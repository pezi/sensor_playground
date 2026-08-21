//! Compact ISL29125 driver, ported from the register access in the Python
//! node (smbus2): the device-ID check and reset, the three configuration
//! registers, and a 6-byte block read of the green/red/blue counts
//! starting at register 0x09 — readings match the Python node. The colour
//! derivation is platform-neutral (and unit tested); only the I2C access
//! is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use common::Payload;
use serde_json::json;

/// Approximate green-counts-to-lux factor for the 10K range at 16 bits.
pub const LUX_PER_COUNT: f64 = 10000.0 / 65535.0;

/// The illuminance the app charts, derived from the green channel — its
/// spectral response resembles the human eye.
pub fn lux_for(green: u16) -> i64 {
    (f64::from(green) * LUX_PER_COUNT).round() as i64
}

/// Turn the raw 16-bit channel counts into the REST payload: the
/// illuminance plus the colour normalized against the brightest channel so
/// the app can show it directly. In complete darkness there is no colour
/// to report, so the red/green/blue keys are absent.
pub fn derive_reading(green: u16, red: u16, blue: u16) -> Payload {
    let mut reading = Payload::new();
    reading.insert("lux".into(), json!(lux_for(green)));
    let brightest = green.max(red).max(blue);
    if brightest > 0 {
        let normalize =
            |channel: u16| (255.0 * f64::from(channel) / f64::from(brightest)).round() as i64;
        reading.insert("red".into(), json!(normalize(red)));
        reading.insert("green".into(), json!(normalize(green)));
        reading.insert("blue".into(), json!(normalize(blue)));
    }
    reading
}

#[cfg(target_os = "linux")]
pub mod hw {
    use common::i2c::I2CDevice;
    use std::thread::sleep;
    use std::time::Duration;

    // -- ISL29125 register map (see the SparkFun library / datasheet) ----

    const REG_DEVICE_ID: u8 = 0x00; // reads 0x7D; writing 0x46 resets the chip
    const REG_CONFIG1: u8 = 0x01;
    const REG_CONFIG2: u8 = 0x02;
    const REG_CONFIG3: u8 = 0x03;
    const REG_GREEN_LOW: u8 = 0x09; // G L/H, R L/H, B L/H — six consecutive bytes

    const DEVICE_ID: u8 = 0x7D;
    const RESET_COMMAND: u8 = 0x46;

    /// CONFIG1: RGB sampling mode (0x05) in the 10,000 lux range (0x08), 16-bit.
    const CONFIG1_RGB_10KLUX: u8 = 0x0D;
    /// CONFIG2: IR compensation on, maximum adjustment (the SparkFun default).
    const CONFIG2_IR_ADJUST_HIGH: u8 = 0xBF;
    const CONFIG3_NO_INTERRUPTS: u8 = 0x00;

    pub struct Isl29125 {
        dev: I2CDevice,
    }

    impl Isl29125 {
        /// Open the sensor on /dev/i2c-<bus>, verify the device ID and
        /// configure RGB sampling.
        pub fn new(bus: u8, address: u16) -> Result<Self, String> {
            let mut dev = I2CDevice::open(bus, address)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let device_id = dev.read_reg(REG_DEVICE_ID).map_err(|e| e.to_string())?;
            if device_id != DEVICE_ID {
                return Err(format!(
                    "No ISL29125 at {address:#04x} (device ID {device_id:#04x}, \
                     expected {DEVICE_ID:#04x})"
                ));
            }
            dev.write_reg(REG_DEVICE_ID, RESET_COMMAND)
                .map_err(|e| e.to_string())?;
            sleep(Duration::from_millis(100));
            for (reg, value) in [
                (REG_CONFIG1, CONFIG1_RGB_10KLUX),
                (REG_CONFIG2, CONFIG2_IR_ADJUST_HIGH),
                (REG_CONFIG3, CONFIG3_NO_INTERRUPTS),
            ] {
                dev.write_reg(reg, value).map_err(|e| e.to_string())?;
            }
            // One conversion takes ~100 ms per channel; let the first RGB
            // sampling cycle complete before serving readings.
            sleep(Duration::from_millis(400));
            Ok(Self { dev })
        }

        /// The raw 16-bit green, red and blue counts.
        pub fn read_channels(&mut self) -> Result<(u16, u16, u16), String> {
            let d = self
                .dev
                .read_regs(REG_GREEN_LOW, 6)
                .map_err(|e| e.to_string())?;
            let word = |lo: usize| u16::from(d[lo]) | u16::from(d[lo + 1]) << 8;
            Ok((word(0), word(2), word(4)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counts → payload derivation must match the Python node: lux from
    /// the green channel, the colour normalized against the brightest one.
    #[test]
    fn normalizes_against_brightest_channel() {
        let reading = derive_reading(30000, 60000, 15000);
        // Red is the brightest channel, so it saturates at 255.
        assert_eq!(reading["red"], 255);
        assert_eq!(reading["green"], 128);
        assert_eq!(reading["blue"], 64);
        // 30000 * 10000/65535 ≈ 4578 lux.
        assert_eq!(reading["lux"], 4578);
    }

    #[test]
    fn full_scale_is_white_at_the_top_of_the_range() {
        let reading = derive_reading(65535, 65535, 65535);
        assert_eq!(reading["lux"], 10000);
        for key in ["red", "green", "blue"] {
            assert_eq!(reading[key], 255);
        }
    }

    /// In complete darkness there is no colour to report.
    #[test]
    fn darkness_has_no_colour() {
        let reading = derive_reading(0, 0, 0);
        assert_eq!(reading["lux"], 0);
        for key in ["red", "green", "blue"] {
            assert!(!reading.contains_key(key), "{key} must be absent");
        }
    }
}
