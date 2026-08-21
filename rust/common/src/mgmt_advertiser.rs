//! BLE advertising via the kernel's Bluetooth management (mgmt) socket.
//! Port of python/common/mgmt_advertiser.py.
//!
//! Workaround for a Raspberry Pi kernel regression: bluetoothd sends
//! ADD_EXT_ADV_DATA with 8 trailing bytes (it sizes the buffer with the
//! legacy `mgmt_cp_add_advertising` header, 11 bytes, instead of the 3-byte
//! extended one), and the RPi kernel rejects the over-long command with
//! Invalid Parameters (0x0d). Every BLE peripheral library is affected, so
//! a sensor node cannot advertise at all through bluetoothd.
//!
//! Registering the GATT server through BlueZ still works, only the
//! advertising step fails. This module performs that one step directly
//! against the kernel with a correctly sized command.
//!
//! Requires CAP_NET_ADMIN, i.e. the node must run as root.

#![cfg(target_os = "linux")]

use std::os::fd::RawFd;

const HCI_CHANNEL_CONTROL: u16 = 3;
const MGMT_INDEX_NONE: u16 = 0xFFFF;

const OP_READ_CONTROLLER_INFO: u16 = 0x0004;
const OP_ADD_EXT_ADV_PARAMS: u16 = 0x0054;
const OP_ADD_EXT_ADV_DATA: u16 = 0x0055;
const OP_ADD_ADVERTISING: u16 = 0x003E;
const OP_REMOVE_ADVERTISING: u16 = 0x003F;

const EV_CMD_COMPLETE: u16 = 0x0001;
const EV_CMD_STATUS: u16 = 0x0002;

const FLAG_CONNECTABLE: u32 = 1 << 0;
const FLAG_DISCOVERABLE: u32 = 1 << 1;
const PARAM_INTERVALS: u32 = 1 << 14;
const PARAM_SCAN_RSP: u32 = 1 << 16;

// Advertising interval in units of 0.625 ms. 100-150 ms costs little power
// and is found on an Android scanner's first duty-cycled scan window.
const MIN_INTERVAL: u32 = 0x00A0; // 100 ms
const MAX_INTERVAL: u32 = 0x00F0; // 150 ms

const AD_UUID128_COMPLETE: u8 = 0x07;
const AD_NAME_COMPLETE: u8 = 0x09;

const INSTANCE: u8 = 1; // adapter-wide: only one BLE node per board
const TIMEOUT_SECS: i64 = 5;

const BTPROTO_HCI: i32 = 1;

#[derive(Debug)]
pub struct MgmtAdvertisingError(pub String);

impl std::fmt::Display for MgmtAdvertisingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for MgmtAdvertisingError {}

/// Whether the kernel is actually transmitting LE advertisements on
/// adapter 0 (MGMT_SETTING_ADVERTISING in the controller's current
/// settings). bluetoothd can hold a registered advertisement instance
/// while the controller flag is off — registered but not on air.
/// Requires CAP_NET_ADMIN; returns Err when the mgmt socket is unavailable.
pub fn kernel_advertising_active() -> Result<bool, MgmtAdvertisingError> {
    let fd = open_mgmt_socket()?;
    // Read Controller Info: response is address(6) version(1)
    // manufacturer(2) supported_settings(4) current_settings(4) ...
    let adv = MgmtAdvertiser {
        index: 0,
        fd,
        name: String::new(),
        service_uuid: String::new(),
        owns_instance: false,
    };
    let info = adv.command_with_response(OP_READ_CONTROLLER_INFO, &[])?;
    if info.len() < 17 {
        return Err(MgmtAdvertisingError("short controller info".into()));
    }
    let current = u32::from_le_bytes([info[13], info[14], info[15], info[16]]);
    const MGMT_SETTING_ADVERTISING: u32 = 1 << 10;
    Ok(current & MGMT_SETTING_ADVERTISING != 0)
}

/// Advertises a service UUID and local name through the mgmt socket.
pub struct MgmtAdvertiser {
    index: u16,
    fd: RawFd,
    name: String,
    service_uuid: String,
    /// False for probe-only instances: Drop must not remove the live
    /// advertising instance someone else registered.
    owns_instance: bool,
}

impl MgmtAdvertiser {
    /// Open the mgmt socket and advertise `name` (scan response) and
    /// `service_uuid` (advertising data) on adapter `index`.
    pub fn start(name: &str, service_uuid: &str) -> Result<Self, MgmtAdvertisingError> {
        let fd = open_mgmt_socket()?;
        let adv = Self {
            index: 0,
            fd,
            name: name.to_string(),
            service_uuid: service_uuid.to_string(),
            owns_instance: true,
        };
        adv.add()?;
        Ok(adv)
    }

    /// Re-arm the instance after a client disconnected: the controller stops
    /// a connectable advertisement once a central connects, and nobody
    /// re-enables ours.
    pub fn restart(&self) -> Result<(), MgmtAdvertisingError> {
        self.add()
    }

    fn add(&self) -> Result<(), MgmtAdvertisingError> {
        let adv_data = uuid128_ad(&self.service_uuid);
        let scan_rsp = name_ad(&self.name);
        let flags = FLAG_CONNECTABLE | FLAG_DISCOVERABLE;

        // A SIGKILLed run leaks its instance, so clear ours before claiming
        // it. This is why only one BLE node per board is supported.
        let _ = self.command(OP_REMOVE_ADVERTISING, &[INSTANCE]);

        match self.add_extended(flags | PARAM_INTERVALS | PARAM_SCAN_RSP, &adv_data, &scan_rsp) {
            Ok(()) => Ok(()),
            Err(_) => {
                // Controllers without extended advertising use the legacy
                // command, which has no interval fields: the kernel then
                // advertises at its 1.28 s default.
                println!("BLE: no extended advertising, falling back to a 1.28 s interval");
                self.add_legacy(flags, &adv_data, &scan_rsp)
            }
        }
    }

    /// Remove the advertising instance.
    pub fn stop(&self) {
        let _ = self.command(OP_REMOVE_ADVERTISING, &[INSTANCE]);
    }

    // -- mgmt plumbing -------------------------------------------------------

    /// Send a mgmt command and return its status byte.
    fn command(&self, opcode: u16, params: &[u8]) -> Result<u8, MgmtAdvertisingError> {
        self.command_full(opcode, params).map(|(status, _)| status)
    }

    /// Send a mgmt command; return its response payload (after the echoed
    /// opcode and status byte), failing on a non-zero status.
    fn command_with_response(
        &self,
        opcode: u16,
        params: &[u8],
    ) -> Result<Vec<u8>, MgmtAdvertisingError> {
        let (status, data) = self.command_full(opcode, params)?;
        if status != 0 {
            return Err(MgmtAdvertisingError(format!(
                "mgmt command 0x{opcode:04x} failed (0x{status:02x})"
            )));
        }
        Ok(data)
    }

    fn command_full(
        &self,
        opcode: u16,
        params: &[u8],
    ) -> Result<(u8, Vec<u8>), MgmtAdvertisingError> {
        let mut packet = Vec::with_capacity(6 + params.len());
        packet.extend_from_slice(&opcode.to_le_bytes());
        packet.extend_from_slice(&self.index.to_le_bytes());
        packet.extend_from_slice(&(params.len() as u16).to_le_bytes());
        packet.extend_from_slice(params);

        let sent = unsafe {
            libc::send(self.fd, packet.as_ptr() as *const libc::c_void, packet.len(), 0)
        };
        if sent < 0 {
            return Err(MgmtAdvertisingError(format!(
                "mgmt send failed (errno {})",
                std::io::Error::last_os_error()
            )));
        }

        let mut buf = [0u8; 1024];
        loop {
            let n = unsafe {
                libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0)
            };
            if n < 6 {
                return Err(MgmtAdvertisingError(format!(
                    "mgmt recv failed ({})",
                    std::io::Error::last_os_error()
                )));
            }
            let n = n as usize;
            let event = u16::from_le_bytes([buf[0], buf[1]]);
            let length = u16::from_le_bytes([buf[4], buf[5]]) as usize;
            let body = &buf[6..(6 + length).min(n)];
            if event != EV_CMD_COMPLETE && event != EV_CMD_STATUS {
                continue; // unrelated event for another client
            }
            if body.len() < 3 {
                continue;
            }
            let echoed = u16::from_le_bytes([body[0], body[1]]);
            if echoed == opcode {
                return Ok((body[2], body[3..].to_vec()));
            }
        }
    }

    fn add_extended(
        &self,
        flags: u32,
        adv_data: &[u8],
        scan_rsp: &[u8],
    ) -> Result<(), MgmtAdvertisingError> {
        // struct: instance u8, flags u32, duration u16, timeout u16,
        //         min_interval u32, max_interval u32, tx_power i8 (packed LE)
        let mut params = Vec::with_capacity(18);
        params.push(INSTANCE);
        params.extend_from_slice(&flags.to_le_bytes());
        params.extend_from_slice(&0u16.to_le_bytes());
        params.extend_from_slice(&0u16.to_le_bytes());
        params.extend_from_slice(&MIN_INTERVAL.to_le_bytes());
        params.extend_from_slice(&MAX_INTERVAL.to_le_bytes());
        params.push(0); // tx_power 0
        let status = self.command(OP_ADD_EXT_ADV_PARAMS, &params)?;
        if status != 0 {
            return Err(MgmtAdvertisingError(format!(
                "add ext adv params failed (0x{status:02x})"
            )));
        }

        let mut data = Vec::with_capacity(3 + adv_data.len() + scan_rsp.len());
        data.push(INSTANCE);
        data.push(adv_data.len() as u8);
        data.push(scan_rsp.len() as u8);
        data.extend_from_slice(adv_data);
        data.extend_from_slice(scan_rsp);
        let status = self.command(OP_ADD_EXT_ADV_DATA, &data)?;
        if status != 0 {
            return Err(MgmtAdvertisingError(format!(
                "add ext adv data failed (0x{status:02x})"
            )));
        }
        Ok(())
    }

    fn add_legacy(
        &self,
        flags: u32,
        adv_data: &[u8],
        scan_rsp: &[u8],
    ) -> Result<(), MgmtAdvertisingError> {
        // struct: instance u8, flags u32, duration u16, timeout u16,
        //         adv_data_len u8, scan_rsp_len u8 (packed LE)
        let mut data = Vec::with_capacity(11 + adv_data.len() + scan_rsp.len());
        data.push(INSTANCE);
        data.extend_from_slice(&flags.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.push(adv_data.len() as u8);
        data.push(scan_rsp.len() as u8);
        data.extend_from_slice(adv_data);
        data.extend_from_slice(scan_rsp);
        let status = self.command(OP_ADD_ADVERTISING, &data)?;
        if status != 0 {
            return Err(MgmtAdvertisingError(format!(
                "add advertising failed (0x{status:02x})"
            )));
        }
        Ok(())
    }
}

impl Drop for MgmtAdvertiser {
    fn drop(&mut self) {
        if self.owns_instance {
            self.stop();
        }
        unsafe { libc::close(self.fd) };
    }
}

fn open_mgmt_socket() -> Result<RawFd, MgmtAdvertisingError> {
    let fd = unsafe {
        libc::socket(
            libc::AF_BLUETOOTH,
            libc::SOCK_RAW | libc::SOCK_CLOEXEC,
            BTPROTO_HCI,
        )
    };
    if fd < 0 {
        return Err(MgmtAdvertisingError(format!(
            "cannot create the Bluetooth management socket ({})",
            std::io::Error::last_os_error()
        )));
    }

    // sockaddr_hci { family u16, dev u16, channel u16 }, bound to the mgmt
    // control channel.
    let addr: [u8; 6] = {
        let mut a = [0u8; 6];
        a[0..2].copy_from_slice(&(libc::AF_BLUETOOTH as u16).to_le_bytes());
        a[2..4].copy_from_slice(&MGMT_INDEX_NONE.to_le_bytes());
        a[4..6].copy_from_slice(&HCI_CHANNEL_CONTROL.to_le_bytes());
        a
    };
    let rc = unsafe {
        libc::bind(
            fd,
            addr.as_ptr() as *const libc::sockaddr,
            addr.len() as libc::socklen_t,
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(MgmtAdvertisingError(format!(
            "cannot open the Bluetooth management socket ({err}); \
             the node must run as root to advertise"
        )));
    }

    let timeout = libc::timeval {
        tv_sec: TIMEOUT_SECS,
        tv_usec: 0,
    };
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_RCVTIMEO,
            &timeout as *const _ as *const libc::c_void,
            std::mem::size_of::<libc::timeval>() as libc::socklen_t,
        );
    }
    Ok(fd)
}

// -- AD structures -----------------------------------------------------------

/// Complete list of 128-bit service UUIDs (little-endian, as BLE wants).
fn uuid128_ad(uuid: &str) -> Vec<u8> {
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    let mut raw: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    raw.reverse();
    let mut ad = vec![raw.len() as u8 + 1, AD_UUID128_COMPLETE];
    ad.extend_from_slice(&raw);
    ad
}

/// Complete local name, trimmed to what one AD structure can hold.
fn name_ad(name: &str) -> Vec<u8> {
    let raw = &name.as_bytes()[..name.len().min(29)];
    let mut ad = vec![raw.len() as u8 + 1, AD_NAME_COMPLETE];
    ad.extend_from_slice(raw);
    ad
}
