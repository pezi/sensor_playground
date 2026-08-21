//! Minimal SHT31 driver, ported from the Python node: single-shot
//! measurement, high repeatability, no clock stretching. The port
//! additionally verifies the Sensirion CRC-8 of both data words (the
//! Python node reads but does not check the CRC bytes). The CRC and
//! conversion math is platform-neutral (and unit tested); only the I2C
//! access is Linux-only.
//!
//! The SHT31 is command-based, not register-mapped: reading a measurement
//! is a plain 6-byte I2C read with no preceding register write, which the
//! common I2CDevice wrapper cannot express — so the hw module opens
//! /dev/i2c-N itself (same I2C_SLAVE ioctl approach as common/src/i2c.rs).

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Sensirion CRC-8 (polynomial 0x31, init 0xFF) that protects each 16-bit
/// word of the measurement frame.
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xff;
    for &b in data {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { crc << 1 ^ 0x31 } else { crc << 1 };
        }
    }
    crc
}

pub fn convert_temperature(raw: u16) -> f64 {
    -45.0 + 175.0 * f64::from(raw) / 65535.0
}

pub fn convert_humidity(raw: u16) -> f64 {
    100.0 * f64::from(raw) / 65535.0
}

/// Check both CRCs of a 6-byte measurement frame (temp msb, temp lsb, crc,
/// hum msb, hum lsb, crc) and convert the raw words with the datasheet
/// formulas: (temperature °C, humidity %RH).
pub fn parse_frame(frame: &[u8]) -> Result<(f64, f64), String> {
    if frame.len() != 6 {
        return Err(format!("short SHT31 frame: {} bytes", frame.len()));
    }
    if crc8(&frame[0..2]) != frame[2] {
        return Err("SHT31 temperature CRC mismatch".into());
    }
    if crc8(&frame[3..5]) != frame[5] {
        return Err("SHT31 humidity CRC mismatch".into());
    }
    let temp_raw = u16::from(frame[0]) << 8 | u16::from(frame[1]);
    let hum_raw = u16::from(frame[3]) << 8 | u16::from(frame[4]);
    Ok((convert_temperature(temp_raw), convert_humidity(hum_raw)))
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    const I2C_SLAVE: libc::c_ulong = 0x0703; // linux/i2c-dev.h
    const ADDRESS: libc::c_ulong = 0x44;

    /// Single-shot measurement command: high repeatability, no clock
    /// stretching (like the Python node).
    const MEASURE_CMD: [u8; 2] = [0x24, 0x00];

    pub struct Sht31 {
        f: File,
    }

    impl Sht31 {
        /// Open the sensor on /dev/i2c-<bus> at 0x44.
        pub fn new(bus: u8) -> Result<Self, String> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, ADDRESS) };
            if rc != 0 {
                return Err(format!(
                    "I2C_SLAVE ioctl for 0x44: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { f })
        }

        /// One single-shot measurement: (temperature °C, humidity %RH).
        pub fn read(&mut self) -> Result<(f64, f64), String> {
            self.f.write_all(&MEASURE_CMD).map_err(|e| e.to_string())?;
            // The Python node's conversion wait (datasheet max 15 ms).
            std::thread::sleep(Duration::from_millis(20));
            let mut frame = [0u8; 6];
            self.f.read_exact(&mut frame).map_err(|e| e.to_string())?;
            parse_frame(&frame)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The SHT3x datasheet CRC example: CRC(0xBEEF) = 0x92.
    #[test]
    fn crc_matches_datasheet_example() {
        assert_eq!(crc8(&[0xbe, 0xef]), 0x92);
    }

    // Conversion formulas (§4.13) at exact raw values.
    #[test]
    fn conversion_formulas() {
        assert!((convert_temperature(0x0000) - -45.0).abs() < 1e-9);
        assert!((convert_temperature(0xffff) - 130.0).abs() < 1e-9);
        assert!((convert_temperature(26214) - 25.0).abs() < 1e-9); // 26214/65535 = 2/5
        assert!((convert_humidity(0x0000) - 0.0).abs() < 1e-9);
        assert!((convert_humidity(0xffff) - 100.0).abs() < 1e-9);
        assert!((convert_humidity(26214) - 40.0).abs() < 1e-9);
    }

    // A valid frame parses; a corrupted CRC or data word is detected.
    #[test]
    fn frame_parsing() {
        let crc = crc8(&[0x66, 0x66]);
        let frame = [0x66, 0x66, crc, 0x66, 0x66, crc];
        let (temp, hum) = parse_frame(&frame).unwrap();
        assert!((temp - 25.0).abs() < 1e-9);
        assert!((hum - 40.0).abs() < 1e-9);

        let mut bad = frame;
        bad[2] ^= 0xff;
        assert!(parse_frame(&bad).unwrap_err().contains("temperature CRC"));
        let mut bad = frame;
        bad[4] ^= 0x01;
        assert!(parse_frame(&bad).unwrap_err().contains("humidity CRC"));
    }
}
