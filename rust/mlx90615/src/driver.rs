//! Compact MLX90615 driver, ported from the register access in the Python
//! node (smbus2): SMBus word reads from RAM register 0x26 (ambient) and
//! 0x27 (object), temperature = raw * 0.02 K - 273.15, bit 15 set marks an
//! error — readings match the Python node. The conversion is
//! platform-neutral (and unit tested); only the I2C access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Convert a raw RAM word to °C, or None when the sensor flags an error
/// (bit 15). The RAM value is the absolute temperature in units of 0.02 K.
pub fn decode_temperature(raw: u16) -> Option<f64> {
    if raw & 0x8000 != 0 {
        return None;
    }
    Some(f64::from(raw) * 0.02 - 273.15)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;

    const REG_AMBIENT: u8 = 0x26;
    const REG_OBJECT: u8 = 0x27;

    pub struct Mlx90615 {
        dev: I2CDevice,
    }

    impl Mlx90615 {
        /// Open the sensor on /dev/i2c-<bus> at 0x5B.
        pub fn new(bus: u8) -> Result<Self, String> {
            let dev =
                I2CDevice::open(bus, 0x5B).map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            Ok(Self { dev })
        }

        /// One temperature in °C, or None when the sensor flagged an error.
        ///
        /// The MLX90615 is a strict SMBus part: the register address and
        /// the data read must be one transfer with a repeated start, which
        /// is what read_word_smbus does — a plain write-then-read would put
        /// a stop condition in between and the sensor would abort.
        fn read_temperature(&mut self, register: u8) -> Result<Option<f64>, String> {
            let raw = self
                .dev
                .read_word_smbus(register)
                .map_err(|e| e.to_string())?;
            Ok(decode_temperature(raw))
        }

        /// The sensor's own ambient temperature and the non-contact object
        /// temperature, both in °C — or None when either channel flagged an
        /// error.
        pub fn read(&mut self) -> Result<Option<(f64, f64)>, String> {
            let ambient = self.read_temperature(REG_AMBIENT)?;
            let object = self.read_temperature(REG_OBJECT)?;
            Ok(ambient.zip(object))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The raw RAM word → °C conversion must match the Python node's.
    #[test]
    fn decodes_temperatures() {
        for (raw, want) in [
            (15000u16, 26.85), // 15000 * 0.02 K = 300.00 K
            (14683, 20.51),    // room temperature
            (0x0000, -273.15), // 0 K — what an empty bus reads back
            (0x7FFF, 382.19),  // largest value without the error flag
        ] {
            let got = decode_temperature(raw).expect("no error flag set");
            assert!(
                (got - want).abs() < 1e-9,
                "decode_temperature({raw:#06x}) = {got}, want {want}"
            );
        }
    }

    /// Bit 15 set: the sensor reports an error, not a temperature.
    #[test]
    fn rejects_the_error_flag() {
        for raw in [0x8000u16, 0xFFFF, 0xBAF6] {
            assert!(
                decode_temperature(raw).is_none(),
                "decode_temperature({raw:#06x}) must be None (bit 15 set)"
            );
        }
    }
}
