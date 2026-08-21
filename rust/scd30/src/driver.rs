//! Compact SCD30 driver, ported from the scd30_i2c Python driver
//! (https://github.com/RequestForCoffee/scd30, MIT) — the library the
//! Python node uses: continuous measurement started at a 2 s interval,
//! data-ready polling, Sensirion CRC-8 (poly 0x31, init 0xFF) on every
//! 16-bit word, and the measurement floats assembled from two big-endian
//! words (MSW first) into an IEEE-754 single. The frame parsing and
//! float decoding are platform-neutral (and unit tested); only the I2C
//! access is Linux-only.
//!
//! The SCD30 has no register map — commands are bare 16-bit big-endian
//! writes (arguments carry their own CRC) and results are bare reads —
//! so the driver talks to /dev/i2c-N directly instead of going through
//! the common crate's register-based I2C helper.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Sensirion CRC-8: polynomial 0x31, init 0xFF (datasheet example:
/// 0xBE 0xEF -> 0x92).
pub fn crc8(data: &[u8]) -> u8 {
    let mut crc: u8 = 0xFF;
    for &b in data {
        crc ^= b;
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

/// Validate the per-word CRCs of a response frame ([msb, lsb, crc] per
/// word) and return the big-endian 16-bit words.
pub fn parse_words(frame: &[u8]) -> Result<Vec<u16>, String> {
    if frame.is_empty() || frame.len() % 3 != 0 {
        return Err(format!(
            "SCD30 frame has {} bytes, want a multiple of 3",
            frame.len()
        ));
    }
    let mut words = Vec::with_capacity(frame.len() / 3);
    for (i, group) in frame.chunks(3).enumerate() {
        if crc8(&group[0..2]) != group[2] {
            return Err(format!("SCD30 CRC mismatch in word {i}"));
        }
        words.push(u16::from(group[0]) << 8 | u16::from(group[1]));
    }
    Ok(words)
}

/// Assemble two big-endian 16-bit words (MSW first) into an IEEE-754
/// single-precision float, as the scd30_i2c driver does.
pub fn decode_float(msw: u16, lsw: u16) -> f64 {
    f64::from(f32::from_bits(u32::from(msw) << 16 | u32::from(lsw)))
}

/// Decode the 18-byte read-measurement frame into
/// (co2 ppm, temperature °C, humidity %RH).
pub fn decode_measurement(frame: &[u8]) -> Result<(f64, f64, f64), String> {
    let words = parse_words(frame)?;
    if words.len() != 6 {
        return Err(format!("SCD30 measurement has {} words, want 6", words.len()));
    }
    Ok((
        decode_float(words[0], words[1]),
        decode_float(words[2], words[3]),
        decode_float(words[4], words[5]),
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

    const SCD30_ADDR: libc::c_ulong = 0x61;

    const CMD_START_PERIODIC: u16 = 0x0010; // arg: ambient pressure mbar, 0 = disabled
    const CMD_SET_INTERVAL: u16 = 0x4600; // arg: measurement interval in seconds
    const CMD_DATA_READY: u16 = 0x0202;
    const CMD_READ_MEASURE: u16 = 0x0300; // 6 words: co2, temperature, humidity

    const INTERVAL: u16 = 2; // seconds, like the Python node

    pub struct Scd30 {
        f: File,
    }

    impl Scd30 {
        /// Open the sensor on /dev/i2c-<bus> at 0x61, set the 2 s
        /// measurement interval and start continuous measurement
        /// (ambient pressure compensation disabled), like the Python
        /// node.
        pub fn new(bus: u8) -> Result<Self, String> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, SCD30_ADDR) };
            if rc != 0 {
                return Err(format!(
                    "I2C_SLAVE ioctl for 0x61: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut s = Self { f };
            // Set the interval, then read back and discard the echoed
            // word (the scd30_i2c driver does the same).
            s.send_command(CMD_SET_INTERVAL, &[INTERVAL])?;
            s.read_words(1)
                .map_err(|e| format!("setting SCD30 measurement interval: {e}"))?;
            s.send_command(CMD_START_PERIODIC, &[0])?;
            Ok(s)
        }

        /// Write a big-endian command word plus CRC-protected arguments,
        /// then wait the >3 ms the datasheet requires between I2C
        /// transactions (the scd30_i2c driver waits 5 ms).
        fn send_command(&mut self, cmd: u16, args: &[u16]) -> Result<(), String> {
            let mut msg = vec![(cmd >> 8) as u8, cmd as u8];
            for &a in args {
                let word = [(a >> 8) as u8, a as u8];
                msg.extend_from_slice(&word);
                msg.push(crc8(&word));
            }
            self.f.write_all(&msg).map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(5));
            Ok(())
        }

        /// Read `n` CRC-protected words from the sensor.
        fn read_words(&mut self, n: usize) -> Result<Vec<u16>, String> {
            let mut frame = vec![0u8; 3 * n];
            self.f.read_exact(&mut frame).map_err(|e| e.to_string())?;
            parse_words(&frame)
        }

        /// Poll the data-ready status word.
        pub fn data_ready(&mut self) -> Result<bool, String> {
            self.send_command(CMD_DATA_READY, &[])?;
            Ok(self.read_words(1)?[0] == 1)
        }

        /// One measurement: (co2 ppm, temperature °C, humidity %RH) once
        /// fresh data is ready, or None while none is (like the Python
        /// node's read() returning None).
        pub fn read(&mut self) -> Result<Option<(f64, f64, f64)>, String> {
            if !self.data_ready()? {
                return Ok(None);
            }
            self.send_command(CMD_READ_MEASURE, &[])?;
            let mut frame = [0u8; 18];
            self.f.read_exact(&mut frame).map_err(|e| e.to_string())?;
            decode_measurement(&frame).map(Some)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The read-measurement example frame from the Sensirion SCD30
    // interface description: CO2 0x43DB8C2E, temperature 0x41D9E7FF,
    // humidity 0x42433A1B.
    const EXAMPLE_FRAME: [u8; 18] = [
        0x43, 0xDB, 0xCB, 0x8C, 0x2E, 0x8F, // CO2 = 439.095 ppm
        0x41, 0xD9, 0x70, 0xE7, 0xFF, 0xF5, // temperature = 27.238 °C
        0x42, 0x43, 0xBF, 0x3A, 0x1B, 0x74, // humidity = 48.806 %RH
    ];

    #[test]
    fn crc8_known_vectors() {
        let cases: [(&[u8], u8); 5] = [
            (&[0xBE, 0xEF], 0x92), // Sensirion datasheet example
            (&[0x00, 0x00], 0x81), // start-periodic argument (pressure 0)
            (&[0x00, 0x02], 0xE3), // set-interval argument (2 s)
            (&[0x43, 0xDB], 0xCB),
            (&[0x8C, 0x2E], 0x8F),
        ];
        for (data, want) in cases {
            assert_eq!(crc8(data), want, "crc8({data:02x?})");
        }
    }

    #[test]
    fn parse_words_validates_crcs() {
        let words = parse_words(&EXAMPLE_FRAME).expect("valid frame rejected");
        assert_eq!(words.len(), 6);
        assert_eq!(words[0], 0x43DB);
        assert_eq!(words[1], 0x8C2E);

        let mut corrupt = EXAMPLE_FRAME;
        corrupt[5] ^= 0x01;
        assert!(parse_words(&corrupt).is_err(), "corrupt CRC accepted");
        assert!(parse_words(&EXAMPLE_FRAME[..4]).is_err(), "short frame accepted");
    }

    // The words are combined MSW-first into a big-endian IEEE-754 single.
    #[test]
    fn float_decoding() {
        assert_eq!(decode_float(0x3F80, 0x0000), 1.0);
        assert_eq!(decode_float(0x0000, 0x0000), 0.0);

        let (co2, temperature, humidity) =
            decode_measurement(&EXAMPLE_FRAME).expect("valid measurement rejected");
        assert!((co2 - 439.09515380859375).abs() < 1e-9, "co2 = {co2}");
        assert!(
            (temperature - 27.238279342651367).abs() < 1e-9,
            "temperature = {temperature}"
        );
        assert!((humidity - 48.80674362182617).abs() < 1e-9, "humidity = {humidity}");

        let mut corrupt = EXAMPLE_FRAME;
        corrupt[5] ^= 0x01;
        assert!(decode_measurement(&corrupt).is_err(), "corrupt measurement accepted");
    }
}
