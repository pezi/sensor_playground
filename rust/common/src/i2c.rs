//! Minimal /dev/i2c-N wrapper (Linux) — plain write/read syscalls after an
//! I2C_SLAVE ioctl, enough for the register-map sensors the nodes use.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;

const I2C_SLAVE: libc::c_ulong = 0x0703;
const I2C_SMBUS: libc::c_ulong = 0x0720;
const I2C_SMBUS_READ: u8 = 1;
const I2C_SMBUS_WORD_DATA: u32 = 3;

/// struct i2c_smbus_ioctl_data (linux/i2c-dev.h).
#[repr(C)]
struct SmbusIoctlData {
    read_write: u8,
    command: u8,
    size: u32,
    data: *mut u8, // union i2c_smbus_data *
}

pub struct I2CDevice {
    f: File,
}

impl I2CDevice {
    pub fn open(bus: u8, addr: u16) -> std::io::Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/dev/i2c-{bus}"))?;
        let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, addr as libc::c_ulong) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { f })
    }

    /// Write a single value to a register.
    pub fn write_reg(&mut self, reg: u8, val: u8) -> std::io::Result<()> {
        self.f.write_all(&[reg, val])
    }

    /// Write a raw byte sequence in one transfer — for devices like the
    /// SSD1306 that take a control byte followed by a whole frame rather
    /// than register/value pairs. The i2c-dev driver caps a single write at
    /// 8192 bytes, well above the 1025 a full display frame needs.
    pub fn write_bytes(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.f.write_all(data)
    }

    /// Read `length` bytes starting at `reg`.
    pub fn read_regs(&mut self, reg: u8, length: usize) -> std::io::Result<Vec<u8>> {
        self.f.write_all(&[reg])?;
        let mut buf = vec![0u8; length];
        self.f.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// Read a single register.
    pub fn read_reg(&mut self, reg: u8) -> std::io::Result<u8> {
        Ok(self.read_regs(reg, 1)?[0])
    }

    /// Read a little-endian 16-bit word at `reg` (SMBus read_word_data).
    pub fn read_word(&mut self, reg: u8) -> std::io::Result<u16> {
        let b = self.read_regs(reg, 2)?;
        Ok(u16::from(b[0]) | u16::from(b[1]) << 8)
    }

    /// Read a little-endian 16-bit word at `reg` through the kernel's SMBus
    /// ioctl — the same call smbus2's read_word_data() makes in the Python
    /// nodes.
    ///
    /// `read_word` above writes the register address and reads the data in
    /// two separate transfers, with a stop condition in between. Strict
    /// SMBus parts such as the MLX90615 reject that: they need a *repeated
    /// start*. Letting the kernel run the SMBus transaction gets one (the
    /// bcm2835 adapter has no SMBus hardware, so the kernel emulates the
    /// protocol with a combined I2C message).
    pub fn read_word_smbus(&mut self, reg: u8) -> std::io::Result<u16> {
        // union i2c_smbus_data is 34 bytes (its block member); a word read
        // fills the first two.
        let mut buf = [0u8; 34];
        let mut args = SmbusIoctlData {
            read_write: I2C_SMBUS_READ,
            command: reg,
            size: I2C_SMBUS_WORD_DATA,
            data: buf.as_mut_ptr(),
        };
        let rc = unsafe { libc::ioctl(self.f.as_raw_fd(), I2C_SMBUS, &mut args) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(u16::from(buf[0]) | u16::from(buf[1]) << 8)
    }
}
