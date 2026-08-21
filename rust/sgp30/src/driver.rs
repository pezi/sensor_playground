//! Compact SGP30 driver, ported from the Pimoroni sgp30-python driver
//! (https://github.com/pimoroni/sgp30-python, MIT) — the library the Python
//! node uses — so the behavior matches it: big-endian 16-bit command words,
//! CRC-8-checked response words, and the blocking warm-up that discards the
//! sensor's initialization readings (fixed 400 ppm / 0 ppb). The CRC and
//! response parsing are platform-neutral (and unit tested); only the I2C
//! access is Linux-only.
//!
//! The SGP30 is not a register-map device — a command is a plain 2-byte
//! write and the response a plain read — so the hardware module opens
//! /dev/i2c-N directly (same syscalls as common::i2c).

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// The 8-bit CRC of a 16-bit word as defined in section 6.6 of the SGP30
/// datasheet: polynomial 0x31 (x8 + x5 + x4 + 1), initialization 0xFF, no
/// reflection, no final XOR.
pub fn crc8(word: u16) -> u8 {
    let mut crc: u8 = 0xff;
    for byte in [(word >> 8) as u8, word as u8] {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                crc << 1 ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// Verify the per-word CRC of a response buffer (each 16-bit word is
/// followed by its CRC byte) and return the words.
pub fn parse_words(buf: &[u8]) -> Result<Vec<u16>, String> {
    let mut words = Vec::with_capacity(buf.len() / 3);
    for chunk in buf.chunks_exact(3) {
        let word = u16::from(chunk[0]) << 8 | u16::from(chunk[1]);
        if chunk[2] != crc8(word) {
            return Err(format!(
                "invalid CRC in response from SGP30: {:02x} != {:02x}",
                chunk[2],
                crc8(word)
            ));
        }
        words.push(word);
    }
    Ok(words)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    const SGP30_ADDR: u16 = 0x58;
    const I2C_SLAVE: libc::c_ulong = 0x0703; // linux/i2c-dev.h

    const CMD_INIT_AIR_QUALITY: u16 = 0x2003; // no response words
    const CMD_MEASURE_AIR_QUALITY: u16 = 0x2008; // 2 response words: eCO2 ppm, TVOC ppb

    pub struct Sgp30 {
        f: File,
    }

    impl Sgp30 {
        /// Open the sensor on /dev/i2c-<bus> at 0x58.
        pub fn new(bus: u8) -> Result<Self, String> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, SGP30_ADDR as libc::c_ulong) };
            if rc != 0 {
                return Err(format!(
                    "I2C_SLAVE ioctl for 0x{SGP30_ADDR:02x}: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(Self { f })
        }

        /// Write a 16-bit command word, wait the reference driver's fixed
        /// 25 ms and read `response_words` CRC-checked 16-bit words back.
        fn command(&mut self, cmd: u16, response_words: usize) -> Result<Vec<u16>, String> {
            self.f
                .write_all(&cmd.to_be_bytes())
                .map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(25));
            if response_words == 0 {
                return Ok(Vec::new());
            }
            let mut buf = vec![0u8; response_words * 3];
            self.f.read_exact(&mut buf).map_err(|e| e.to_string())?;
            parse_words(&buf)
        }

        /// Start air quality measurement, mirroring the reference driver's
        /// start_measurement: after init_air_quality the SGP30 returns
        /// fixed 400 ppm / 0 ppb readings for about 15 s (page 8/15 of the
        /// datasheet), so readings are discarded until they change —
        /// capped at 20 test samples to avoid a potential infinite loop.
        pub fn start_measurement(&mut self) -> Result<(), String> {
            self.command(CMD_INIT_AIR_QUALITY, 0)?;
            let mut test_samples = 0;
            loop {
                let words = self.command(CMD_MEASURE_AIR_QUALITY, 2)?;
                if words[0] != 400 || words[1] != 0 || test_samples >= 20 {
                    return Ok(());
                }
                std::thread::sleep(Duration::from_secs(1));
                test_samples += 1;
            }
        }

        /// One air-quality measurement: (eCO2 ppm, TVOC ppb).
        pub fn read(&mut self) -> Result<(u16, u16), String> {
            let words = self.command(CMD_MEASURE_AIR_QUALITY, 2)?;
            Ok((words[0], words[1]))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The CRC-8 is checked against the SGP30 datasheet example
    // (0xBEEF -> 0x92, section 6.6) and vectors generated with the
    // Pimoroni sgp30-python reference driver.
    #[test]
    fn crc8_matches_reference_vectors() {
        let cases = [
            (0x0000u16, 0x81u8),
            (0xbeef, 0x92), // datasheet example
            (0x1234, 0x37),
            (0x0190, 0x4c), // 400 ppm, the warm-up eCO2 value
            (0x8000, 0xa2),
            (0xffff, 0xac),
        ];
        for (word, crc) in cases {
            assert_eq!(crc8(word), crc, "crc8(0x{word:04x})");
        }
    }

    #[test]
    fn parse_words_verifies_crc() {
        // A valid measure_air_quality warm-up response: 400 ppm, 0 ppb.
        let words = parse_words(&[0x01, 0x90, 0x4c, 0x00, 0x00, 0x81]).unwrap();
        assert_eq!(words, vec![400, 0]);

        // A corrupted CRC must be rejected.
        assert!(parse_words(&[0x01, 0x90, 0x4d]).is_err());
    }
}
