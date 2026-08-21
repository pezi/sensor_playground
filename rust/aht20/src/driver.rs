//! Compact AHT10/AHT20 driver, ported from the Python node's smbus2 code.
//!
//! Protocol (identical on both chips): trigger a measurement with
//! 0xAC 0x33 0x00, wait ~80 ms, then read 6 bytes — a status byte followed
//! by 20-bit humidity and 20-bit temperature. Only the calibrate opcode
//! differs (AHT20: 0xBE, AHT10: 0xE1), so initialization tries both.
//! The raw-to-value conversion is platform-neutral (and unit tested);
//! only the I2C access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Measurement not finished yet (bit 7 of the status byte).
pub const STATUS_BUSY: u8 = 0x80;

/// Raw 6-byte measurement (status, 20-bit humidity, 20-bit temperature)
/// -> (temperature °C, humidity %RH), or None while the sensor is busy.
pub fn parse_reading(raw: &[u8]) -> Option<(f64, f64)> {
    if raw[0] & STATUS_BUSY != 0 {
        return None;
    }
    let hum_raw = u32::from(raw[1]) << 12 | u32::from(raw[2]) << 4 | u32::from(raw[3] >> 4);
    let temp_raw = u32::from(raw[3] & 0x0f) << 16 | u32::from(raw[4]) << 8 | u32::from(raw[5]);
    Some((
        f64::from(temp_raw) / 1048576.0 * 200.0 - 50.0,
        f64::from(hum_raw) / 1048576.0 * 100.0,
    ))
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    const I2C_SLAVE: libc::c_ulong = 0x0703; // linux/i2c-dev.h
    const ADDRESS: libc::c_ulong = 0x38; // fixed on both chips
    const CMD_CALIBRATE_AHT20: [u8; 3] = [0xbe, 0x08, 0x00];
    const CMD_CALIBRATE_AHT10: [u8; 3] = [0xe1, 0x08, 0x00];
    const CMD_MEASURE: [u8; 3] = [0xac, 0x33, 0x00];

    /// The chips speak raw command writes and plain reads (no register
    /// map), so the driver opens /dev/i2c-N itself instead of using the
    /// register-oriented common::i2c::I2CDevice wrapper.
    pub struct Aht20 {
        f: File,
    }

    impl Aht20 {
        /// Open the sensor on /dev/i2c-<bus> at 0x38 and calibrate it.
        pub fn new(bus: u8) -> Result<Self, String> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, ADDRESS) };
            if rc != 0 {
                return Err(format!(
                    "I2C_SLAVE ioctl for 0x38: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut dev = Self { f };
            std::thread::sleep(Duration::from_millis(40));
            dev.calibrate();
            Ok(dev)
        }

        /// Send the calibrate command; tolerate either chip variant.
        fn calibrate(&mut self) {
            for command in [CMD_CALIBRATE_AHT20, CMD_CALIBRATE_AHT10] {
                if self.f.write_all(&command).is_ok() {
                    std::thread::sleep(Duration::from_millis(10));
                    return;
                }
            }
        }

        /// One triggered measurement: (temperature °C, humidity %RH),
        /// or None while the sensor is busy.
        pub fn read(&mut self) -> Result<Option<(f64, f64)>, String> {
            self.f.write_all(&CMD_MEASURE).map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(80));
            let mut raw = [0u8; 6];
            self.f.read_exact(&mut raw).map_err(|e| e.to_string())?;
            Ok(parse_reading(&raw))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Golden values computed with the Python node's formulas (20-bit
    // humidity / 20-bit temperature split across the shared nibble in
    // byte 3). The AHT10/AHT20 protocol the Python node uses reads
    // 6 bytes and has no CRC.
    #[test]
    fn conversion_matches_python_goldens() {
        let cases: [([u8; 6], f64, f64); 5] = [
            // All-zero raw values -> scale minimum.
            ([0x1c, 0x00, 0x00, 0x00, 0x00, 0x00], -50.0, 0.0),
            // Mid-scale on both channels (0x80000 of 0x100000).
            ([0x1c, 0x80, 0x00, 0x08, 0x00, 0x00], 50.0, 50.0),
            // All-ones raw values -> scale maximum.
            ([0x1c, 0xff, 0xff, 0xff, 0xff, 0xff], 149.99980926513672, 99.99990463256836),
            // Realistic indoor reading, shared nibble in byte 3 non-zero.
            ([0x1c, 0x6e, 0x14, 0x85, 0xc7, 0x2a], 22.224807739257812, 43.000030517578125),
            // AHT10-style status byte (calibrated bit only).
            ([0x08, 0x5d, 0x2e, 0x95, 0x8c, 0x51], 19.35138702392578, 36.399173736572266),
        ];
        for (raw, t, h) in cases {
            let (temperature, humidity) = parse_reading(&raw).expect("not busy");
            assert!((temperature - t).abs() < 1e-12, "temperature");
            assert!((humidity - h).abs() < 1e-12, "humidity");
        }
    }

    // A set busy bit (0x80 in the status byte) must yield no reading,
    // like the Python node returning None.
    #[test]
    fn busy_status_yields_none() {
        assert!(parse_reading(&[0x80, 0x12, 0x34, 0x56, 0x78, 0x9a]).is_none());
    }
}
