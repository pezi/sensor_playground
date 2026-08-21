//! Compact TSL2591 driver — a port of the `adafruit_tsl2591` library the
//! Python node uses: the same device-ID check, the same gain and
//! integration-time encoding, and the same two-equation lux formula from
//! the Adafruit Arduino library, so the values match the Python node.
//!
//! Every register access ORs the address with the command bit 0xA0
//! (command + normal operation), exactly as the Python library does.
//!
//! The lux maths is platform-neutral (and unit tested); only the I2C
//! access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// The ADC is 16-bit, but at the shortest integration time it only counts
/// to 0x8FFF.
const MAX_COUNT_100MS: u16 = 0x8FFF;
const MAX_COUNT: u16 = 0xFFFF;

/// Lux equation coefficients (Adafruit Arduino library).
const LUX_DF: f64 = 408.0;
const LUX_COEF_B: f64 = 1.64;
const LUX_COEF_C: f64 = 0.59;
const LUX_COEF_D: f64 = 0.86;

/// Counts at which the current gain is judged too high or too low.
pub const SATURATION_COUNTS: u16 = 0xFFFF;
pub const TOO_DARK_COUNTS: u16 = 100;

/// Gain names accepted in config.json, in ascending order; the index is the
/// position in GAIN_REGISTERS/GAIN_FACTORS.
pub const GAIN_NAMES: [&str; 4] = ["low", "med", "high", "max"];
pub const GAIN_REGISTERS: [u8; 4] = [0x00, 0x10, 0x20, 0x30];
pub const GAIN_FACTORS: [f64; 4] = [1.0, 25.0, 428.0, 9876.0];

/// Resolve a config gain name to its index.
pub fn gain_index_for(name: &str) -> Result<usize, String> {
    GAIN_NAMES
        .iter()
        .position(|candidate| *candidate == name)
        .ok_or_else(|| format!("gain must be one of low, med, high, max, got {name:?}"))
}

/// Map an integration time in milliseconds to the register value (0..5);
/// the chip only supports these six.
pub fn integration_register_for(milliseconds: u32) -> Result<u8, String> {
    match milliseconds {
        100 => Ok(0),
        200 => Ok(1),
        300 => Ok(2),
        400 => Ok(3),
        500 => Ok(4),
        600 => Ok(5),
        other => Err(format!(
            "integration_ms must be one of 100, 200, 300, 400, 500, 600, got {other}"
        )),
    }
}

/// Convert the two raw channel counts to lux, given the integration time
/// register value and the gain factor. None when a channel saturated: the
/// counts still show the app that it is very bright, but the lux value
/// would be wrong.
pub fn calculate_lux(
    channel0: u16,
    channel1: u16,
    integration_register: u8,
    gain: f64,
) -> Option<f64> {
    let atime = 100.0 * f64::from(integration_register) + 100.0;
    let max_counts = if integration_register == 0 {
        MAX_COUNT_100MS
    } else {
        MAX_COUNT
    };
    if channel0 >= max_counts || channel1 >= max_counts {
        return None;
    }
    let (c0, c1) = (f64::from(channel0), f64::from(channel1));
    let cpl = (atime * gain) / LUX_DF;
    // Two approximations of the visible response; the library takes
    // whichever is larger.
    let lux1 = (c0 - LUX_COEF_B * c1) / cpl;
    let lux2 = (LUX_COEF_C * c0 - LUX_COEF_D * c1) / cpl;
    Some(lux1.max(lux2))
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;

    // -- TSL2591 register map ---------------------------------------------

    const ADDRESS: u16 = 0x29;
    const COMMAND_BIT: u8 = 0xA0; // every register access is OR'd with this

    const REG_ENABLE: u8 = 0x00;
    const REG_CONTROL: u8 = 0x01;
    const REG_DEVICE_ID: u8 = 0x12;
    const REG_CHAN0_LOW: u8 = 0x14; // broadband, then channel 1 at 0x16

    const ENABLE_POWER_ON: u8 = 0x01;
    const ENABLE_AEN: u8 = 0x02; // ALS enable

    const DEVICE_ID: u8 = 0x50;

    pub struct Tsl2591 {
        dev: I2CDevice,
        gain_index: usize,
        integration_register: u8,
        auto_gain: bool,
    }

    impl Tsl2591 {
        /// Open the sensor on /dev/i2c-<bus> at 0x29, verify the device ID
        /// and program the configured gain and integration time.
        pub fn new(
            bus: u8,
            gain: &str,
            integration_ms: u32,
            auto_gain: bool,
        ) -> Result<Self, String> {
            let gain_index = gain_index_for(gain)?;
            let integration_register = integration_register_for(integration_ms)?;
            let dev = I2CDevice::open(bus, ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let mut sensor = Self {
                dev,
                gain_index,
                integration_register,
                auto_gain,
            };
            let device_id = sensor.read_u8(REG_DEVICE_ID)?;
            if device_id != DEVICE_ID {
                return Err(format!(
                    "No TSL2591 at {ADDRESS:#04x} (device ID {device_id:#04x}, \
                     expected {DEVICE_ID:#04x})"
                ));
            }
            sensor.apply_gain()?;
            sensor.apply_integration()?;
            // Power on and enable the ALS; the chip then integrates
            // continuously.
            sensor.write_u8(REG_ENABLE, ENABLE_POWER_ON | ENABLE_AEN)?;
            Ok(sensor)
        }

        fn read_u8(&mut self, reg: u8) -> Result<u8, String> {
            self.dev
                .read_reg(COMMAND_BIT | reg)
                .map_err(|e| e.to_string())
        }

        fn write_u8(&mut self, reg: u8, value: u8) -> Result<(), String> {
            self.dev
                .write_reg(COMMAND_BIT | reg, value)
                .map_err(|e| e.to_string())
        }

        /// Write the gain bits, leaving the integration bits alone.
        fn apply_gain(&mut self) -> Result<(), String> {
            let control = self.read_u8(REG_CONTROL)?;
            self.write_u8(
                REG_CONTROL,
                control & 0b1100_1111 | GAIN_REGISTERS[self.gain_index],
            )
        }

        /// Write the integration bits, leaving the gain alone.
        fn apply_integration(&mut self) -> Result<(), String> {
            let control = self.read_u8(REG_CONTROL)?;
            self.write_u8(
                REG_CONTROL,
                control & 0b1111_1000 | self.integration_register,
            )
        }

        /// The broadband (visible + IR) and infrared counts.
        pub fn raw_luminosity(&mut self) -> Result<(u16, u16), String> {
            let d = self
                .dev
                .read_regs(COMMAND_BIT | REG_CHAN0_LOW, 4)
                .map_err(|e| e.to_string())?;
            Ok((
                u16::from(d[0]) | u16::from(d[1]) << 8,
                u16::from(d[2]) | u16::from(d[3]) << 8,
            ))
        }

        /// Lux for the two counts at the currently programmed settings.
        pub fn lux(&self, broadband: u16, infrared: u16) -> Option<f64> {
            calculate_lux(
                broadband,
                infrared,
                self.integration_register,
                GAIN_FACTORS[self.gain_index],
            )
        }

        /// Move one gain step when the broadband channel pins at either end.
        ///
        /// Only one step per reading: changing the gain invalidates the
        /// integration already in flight, so the *next* reading is the one
        /// that benefits. Jumping straight to the extreme instead would
        /// make the value oscillate whenever the light sits near a
        /// threshold.
        pub fn auto_gain_step(&mut self, broadband: u16) {
            if !self.auto_gain {
                return;
            }
            let index = if broadband >= SATURATION_COUNTS {
                self.gain_index.saturating_sub(1)
            } else if broadband <= TOO_DARK_COUNTS {
                (self.gain_index + 1).min(GAIN_FACTORS.len() - 1)
            } else {
                self.gain_index
            };
            if index != self.gain_index {
                self.gain_index = index;
                if let Err(err) = self.apply_gain() {
                    println!("Adjusting gain failed: {err}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lux equation must match the Python node's. The expected values
    /// come from running the reference implementation on the same counts.
    #[test]
    fn lux_equation() {
        for (c0, c1, integration, gain, want) in [
            (5400u16, 1500u16, 2u8, 25.0, 159.936), // 300 ms, med — the defaults
            (200, 60, 0, 1.0, 414.5280000000001),   // 100 ms, low — bright sun
            (37000, 12000, 1, 428.0, 82.55327102803739), // 200 ms, high
            (100, 0, 5, 9876.0, 0.00688537869582827), // 600 ms, max — near dark
        ] {
            let got = calculate_lux(c0, c1, integration, gain).expect("not saturated");
            assert!(
                (got - want).abs() < 1e-9,
                "calculate_lux({c0},{c1},{integration},{gain}) = {got}, want {want}"
            );
        }
    }

    /// At the shortest integration time the ADC only counts to 0x8FFF, so
    /// the same count means saturation at 100 ms but not at 300 ms.
    #[test]
    fn saturation_depends_on_the_integration_time() {
        assert!(calculate_lux(0x8FFF, 100, 0, 1.0).is_none());
        let lux = calculate_lux(0x8FFF, 100, 2, 25.0).expect("not saturated at 300 ms");
        assert!((lux - 1996.4256).abs() < 1e-9, "0x8FFF at 300 ms = {lux}");
        assert!(
            calculate_lux(1000, 0xFFFF, 2, 25.0).is_none(),
            "infrared saturated"
        );
    }

    #[test]
    fn config_values_are_validated() {
        for (index, name) in GAIN_NAMES.iter().enumerate() {
            assert_eq!(gain_index_for(name), Ok(index));
        }
        assert!(gain_index_for("medium").is_err());
        assert_eq!(integration_register_for(300), Ok(2));
        assert!(integration_register_for(250).is_err());
    }
}
