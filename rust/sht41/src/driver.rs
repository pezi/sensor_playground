//! Compact SHT41 driver (single-shot mode), ported from the Python node:
//! high-precision measurement command 0xFD (~8 ms), then a 6-byte frame
//! [temp msb, temp lsb, crc, hum msb, hum lsb, crc] converted with the
//! datasheet formulas. Unlike the Python node this port also validates the
//! two Sensirion CRC-8 checksums (poly 0x31, init 0xFF). The CRC and
//! conversion math is platform-neutral (and unit tested); only the I2C
//! access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Sensirion CRC-8: polynomial 0x31, init 0xFF (datasheet example:
/// 0xBE 0xEF -> 0x92).
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

/// Validate the CRCs of a 6-byte measurement frame and return the raw
/// temperature and humidity words.
pub fn parse_frame(frame: &[u8]) -> Result<(u16, u16), String> {
    if frame.len() != 6 {
        return Err(format!("SHT41 frame has {} bytes, want 6", frame.len()));
    }
    if crc8(&frame[0..2]) != frame[2] || crc8(&frame[3..5]) != frame[5] {
        return Err("SHT41 CRC mismatch".into());
    }
    let temp_raw = u16::from(frame[0]) << 8 | u16::from(frame[1]);
    let hum_raw = u16::from(frame[3]) << 8 | u16::from(frame[4]);
    Ok((temp_raw, hum_raw))
}

/// Convert the raw words with the datasheet formulas the Python node uses:
/// (temperature °C, humidity %RH clamped to 0..100).
pub fn convert(temp_raw: u16, hum_raw: u16) -> (f64, f64) {
    let temperature = -45.0 + 175.0 * f64::from(temp_raw) / 65535.0;
    let humidity = (-6.0 + 125.0 * f64::from(hum_raw) / 65535.0).clamp(0.0, 100.0);
    (temperature, humidity)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    const ADDR: u16 = 0x44;
    const MEASURE_HIGH: u8 = 0xfd; // high-precision single-shot, ~8 ms

    const I2C_SLAVE: libc::c_ulong = 0x0703;

    /// The SHT41 has no register map — the measurement command is a bare
    /// single-byte write and the result a bare 6-byte read (which the
    /// sensor NACKs while still measuring), so the driver talks to
    /// /dev/i2c-N directly instead of going through the common crate's
    /// register-based I2C helper.
    pub struct Sht41 {
        f: File,
    }

    impl Sht41 {
        /// Open the sensor on /dev/i2c-<bus> at 0x44.
        pub fn new(bus: u8) -> Result<Self, String> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, ADDR as libc::c_ulong) };
            if rc != 0 {
                return Err(format!(
                    "I2C_SLAVE ioctl for 0x{ADDR:02x}: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { f })
        }

        /// One high-precision single-shot measurement:
        /// (temperature °C, humidity %RH).
        pub fn read(&mut self) -> Result<(f64, f64), String> {
            self.f
                .write_all(&[MEASURE_HIGH])
                .map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(10));
            let mut frame = [0u8; 6];
            self.f.read_exact(&mut frame).map_err(|e| e.to_string())?;
            let (temp_raw, hum_raw) = parse_frame(&frame)?;
            Ok(convert(temp_raw, hum_raw))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Sensirion CRC-8 known vectors (0xBE 0xEF -> 0x92 is the datasheet
    // example).
    #[test]
    fn crc8_known_vectors() {
        assert_eq!(crc8(&[0xbe, 0xef]), 0x92);
        assert_eq!(crc8(&[0x00, 0x00]), 0x81);
        assert_eq!(crc8(&[0x66, 0x66]), 0x93);
        assert_eq!(crc8(&[0x80, 0x00]), 0xa2);
        assert_eq!(crc8(&[0xff, 0xff]), 0xac);
    }

    #[test]
    fn frame_parsing_validates_crcs() {
        assert_eq!(
            parse_frame(&[0x66, 0x66, 0x93, 0x80, 0x00, 0xa2]),
            Ok((0x6666, 0x8000))
        );
        assert!(parse_frame(&[0x66, 0x66, 0x94, 0x80, 0x00, 0xa2]).is_err());
        assert!(parse_frame(&[0x66, 0x66, 0x93, 0x80, 0x00, 0xa3]).is_err());
        assert!(parse_frame(&[0x66, 0x66, 0x93]).is_err());
    }

    // Datasheet conversion formulas, incl. the humidity clamp to 0..100.
    // 0x6666/0xFFFF = 0.4 exactly, so 25.0 °C / 44.0 %RH fall out exactly.
    #[test]
    fn conversions_match_reference() {
        let near = |a: f64, b: f64| (a - b).abs() < 1e-9;
        let (t, h) = convert(0x0000, 0x0000);
        assert!(near(t, -45.0) && near(h, 0.0)); // humidity -6 clamped to 0
        let (t, h) = convert(0xffff, 0xffff);
        assert!(near(t, 130.0) && near(h, 100.0)); // humidity 119 clamped to 100
        let (t, h) = convert(0x6666, 0x6666);
        assert!(near(t, 25.0) && near(h, 44.0));
        let (t, h) = convert(0x8000, 0x8000);
        assert!(near(t, 42.50133516441596));
        assert!(near(h, 56.50095368886854));
    }
}
