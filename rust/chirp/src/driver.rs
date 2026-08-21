//! Compact Chirp driver, ported from the register access in the Python
//! node (smbus2) — same register map as the Arduino reference library
//! (https://github.com/Apollon77/I2CSoilMoistureSensor): a reset at
//! startup, 16-bit big-endian reads of the capacitance, temperature and
//! light registers, and the light measurement started by writing register
//! 0x03. The capacitance mapping and the temperature decoding are
//! platform-neutral (and unit tested); only the I2C access is Linux-only.
//!
//! The chip answers a register read as a separate transaction after a
//! short pause (the Arduino library waits 20 ms), so the driver opens
//! /dev/i2c-N itself instead of using the combined-transaction
//! common::i2c::I2CDevice wrapper — and it needs the plain one-byte
//! writes (SMBus "send byte") that the reset and light commands are.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Maps a raw capacitance onto 0-100 % between the dry and wet
/// calibration points (the capacitance rises with moisture, so
/// wet > dry); values outside the calibrated span clamp to 0/100 %.
pub fn moisture_percent(capacitance: i64, cap_dry: i64, cap_wet: i64) -> i64 {
    let span = (cap_wet - cap_dry) as f64;
    let p = 100.0 * (capacitance - cap_dry) as f64 / span;
    p.clamp(0.0, 100.0).round() as i64
}

/// Assembles the chip's big-endian 16-bit register value.
pub fn decode_word(hi: u8, lo: u8) -> u16 {
    u16::from(hi) << 8 | u16::from(lo)
}

/// Converts the raw temperature register word to °C: the chip reports a
/// signed 16-bit value in tenths of a degree.
pub fn decode_temperature(word: u16) -> f64 {
    common::round1(f64::from(word as i16) / 10.0)
}

/// Turns the raw light register value into brightness counts: the chip
/// times a phototransistor discharge and so counts *up* in darkness,
/// which the node inverts (higher = brighter).
pub fn light_counts(raw: u16) -> i64 {
    65535 - i64::from(raw)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    const I2C_SLAVE: libc::c_ulong = 0x0703; // linux/i2c-dev.h

    const REG_GET_CAPACITANCE: u8 = 0x00;
    const REG_MEASURE_LIGHT: u8 = 0x03;
    const REG_GET_LIGHT: u8 = 0x04;
    const REG_GET_TEMPERATURE: u8 = 0x05;
    const REG_RESET: u8 = 0x06;
    const REG_GET_VERSION: u8 = 0x07;

    /// A light measurement takes up to three seconds on the chip.
    const LIGHT_MEASURE_TIME: Duration = Duration::from_secs(3);
    /// The chip needs a moment between the register write and the read;
    /// the Arduino reference library waits 20 ms (the Python node instead
    /// gets one combined SMBus transaction from smbus2).
    const READ_DELAY: Duration = Duration::from_millis(20);
    /// The chip needs a moment after a reset.
    const RESET_DELAY: Duration = Duration::from_secs(1);

    /// One set of readings; `light` stays None until the first (up to
    /// three seconds long) light measurement has completed.
    pub struct Reading {
        pub capacitance: i64,
        pub temperature: f64,
        pub light: Option<i64>,
    }

    pub struct Chirp {
        f: File,
        light: Option<i64>,
        light_started: Option<Instant>,
    }

    impl Chirp {
        /// Open the sensor on /dev/i2c-<bus>, reset it and report the
        /// firmware version.
        pub fn new(bus: u8, address: u16) -> Result<Self, String> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, address as libc::c_ulong) };
            if rc != 0 {
                return Err(format!(
                    "I2C_SLAVE ioctl for 0x{address:02x}: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut dev = Self {
                f,
                light: None,
                light_started: None,
            };
            dev.write_byte(REG_RESET).map_err(|e| e.to_string())?;
            std::thread::sleep(RESET_DELAY);
            let version = dev.read_u16(REG_GET_VERSION).map_err(|e| e.to_string())? & 0xff;
            println!("Chirp sensor at 0x{address:02x}, firmware version 0x{version:02x}");
            Ok(dev)
        }

        /// Send a bare command byte (SMBus "send byte").
        fn write_byte(&mut self, value: u8) -> std::io::Result<()> {
            self.f.write_all(&[value])
        }

        /// Read a big-endian 16-bit register.
        fn read_u16(&mut self, register: u8) -> std::io::Result<u16> {
            self.f.write_all(&[register])?;
            std::thread::sleep(READ_DELAY);
            let mut buf = [0u8; 2];
            self.f.read_exact(&mut buf)?;
            Ok(decode_word(buf[0], buf[1]))
        }

        /// Harvest a finished light measurement and start the next one.
        /// The measurement runs on the chip, so this never blocks; the
        /// first call only starts one and leaves the value None.
        fn update_light(&mut self) -> std::io::Result<()> {
            let now = Instant::now();
            if let Some(started) = self.light_started {
                if now.duration_since(started) >= LIGHT_MEASURE_TIME {
                    let raw = self.read_u16(REG_GET_LIGHT)?;
                    self.light = Some(light_counts(raw));
                    self.light_started = None;
                }
            }
            if self.light_started.is_none() {
                self.write_byte(REG_MEASURE_LIGHT)?;
                self.light_started = Some(now);
            }
            Ok(())
        }

        /// One set of readings.
        pub fn read(&mut self) -> Result<Reading, String> {
            self.update_light().map_err(|e| e.to_string())?;
            let capacitance = self
                .read_u16(REG_GET_CAPACITANCE)
                .map_err(|e| e.to_string())?;
            let raw = self
                .read_u16(REG_GET_TEMPERATURE)
                .map_err(|e| e.to_string())?;
            Ok(Reading {
                capacitance: i64::from(capacitance),
                temperature: decode_temperature(raw),
                light: self.light,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The linear calibration mapping: cap_dry -> 0 %, cap_wet -> 100 %
    // (the raw capacitance rises with moisture, so wet > dry).
    #[test]
    fn percent_maps_between_calibration_points() {
        assert_eq!(moisture_percent(290, 290, 520), 0); // dry calibration point
        assert_eq!(moisture_percent(520, 290, 520), 100); // wet calibration point
        assert_eq!(moisture_percent(405, 290, 520), 50); // midpoint
        assert_eq!(moisture_percent(350, 290, 520), 26); // rounds to nearest percent
        assert_eq!(moisture_percent(300, 250, 600), 14); // other calibration points
    }

    #[test]
    fn percent_clamps_outside_span() {
        assert_eq!(moisture_percent(200, 290, 520), 0); // drier than dry
        assert_eq!(moisture_percent(700, 290, 520), 100); // wetter than wet
        assert_eq!(moisture_percent(600, 250, 600), 100);
    }

    // The chip's registers are big-endian 16-bit words.
    #[test]
    fn decode_word_is_big_endian() {
        assert_eq!(decode_word(0x00, 0x00), 0);
        assert_eq!(decode_word(0x01, 0x2c), 300);
        assert_eq!(decode_word(0x02, 0x08), 520);
        assert_eq!(decode_word(0xff, 0x9c), 0xff9c);
        assert_eq!(decode_word(0xff, 0xff), 65535);
    }

    // The temperature register is a signed 16-bit value in tenths of a
    // degree (two's complement, like the Python node's `raw -= 0x10000`).
    #[test]
    fn decode_positive_temperatures() {
        assert_eq!(decode_temperature(0x0000), 0.0);
        assert_eq!(decode_temperature(0x0001), 0.1);
        assert_eq!(decode_temperature(0x00d5), 21.3);
        assert_eq!(decode_temperature(0x0d80), 345.6);
        assert_eq!(decode_temperature(0x7fff), 3276.7); // largest positive value
    }

    #[test]
    fn decode_negative_temperatures() {
        assert_eq!(decode_temperature(0xffff), -0.1); // -1 -> -0.1 °C
        assert_eq!(decode_temperature(0xff9c), -10.0);
        assert_eq!(decode_temperature(0xfec4), -31.6);
        assert_eq!(decode_temperature(0x8000), -3276.8); // most negative value
    }

    // The chip counts a phototransistor discharge *up* in darkness, so
    // the node inverts the raw value into brightness counts.
    #[test]
    fn light_is_inverted() {
        assert_eq!(light_counts(0), 65535); // brightest
        assert_eq!(light_counts(65535), 0); // darkest
        assert_eq!(light_counts(20000), 45535); // mid-scale emulation value
    }
}
