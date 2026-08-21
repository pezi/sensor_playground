//! Compact MCP9808 driver, ported from the register access in the Python
//! node (smbus2): a 2-byte read of the ambient temperature register 0x05,
//! decoded as a 12-bit + sign two's-complement value in units of 1/16 °C
//! (the alert flag bits 15..13 are masked off) — readings match the Python
//! node. The decoding is platform-neutral (and unit tested); only the I2C
//! access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Convert the raw 16-bit ambient temperature register word to °C: lower
/// 12 bits are the magnitude in 1/16 °C, bit 12 is the sign (two's
/// complement), bits 15..13 are alert flags and ignored.
pub fn decode_temperature(word: u16) -> f64 {
    let mut temperature = f64::from(word & 0x0fff) / 16.0;
    if word & 0x1000 != 0 {
        temperature -= 256.0;
    }
    temperature
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;

    const REG_AMBIENT_TEMP: u8 = 0x05;

    pub struct Mcp9808 {
        dev: I2CDevice,
    }

    impl Mcp9808 {
        /// Open the sensor on /dev/i2c-<bus> at 0x18.
        pub fn new(bus: u8) -> Result<Self, String> {
            let dev = I2CDevice::open(bus, 0x18)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            Ok(Self { dev })
        }

        /// One ambient temperature measurement in °C.
        pub fn read(&mut self) -> Result<f64, String> {
            let b = self
                .dev
                .read_regs(REG_AMBIENT_TEMP, 2)
                .map_err(|e| e.to_string())?;
            Ok(decode_temperature(u16::from(b[0]) << 8 | u16::from(b[1])))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The raw-word → °C conversion must match the Python node's decoding:
    // 12-bit magnitude in 1/16 °C, bit 12 sign (two's complement), alert
    // flag bits 15..13 ignored.
    #[test]
    fn decode_positive_temperatures() {
        assert_eq!(decode_temperature(0x0000), 0.0);
        assert_eq!(decode_temperature(0x0001), 0.0625); // one LSB
        assert_eq!(decode_temperature(0x0194), 25.25); // datasheet example
        assert_eq!(decode_temperature(0x0fff), 255.9375); // largest magnitude
    }

    #[test]
    fn decode_negative_temperatures() {
        assert_eq!(decode_temperature(0x1fff), -0.0625); // -1 LSB
        assert_eq!(decode_temperature(0x1fe8), -1.5);
        assert_eq!(decode_temperature(0x1e70), -25.0);
        assert_eq!(decode_temperature(0x1000), -256.0);
    }

    #[test]
    fn decode_masks_alert_flag_bits() {
        assert_eq!(decode_temperature(0xe194), 25.25);
    }
}
