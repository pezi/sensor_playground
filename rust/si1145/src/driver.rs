//! Compact SI1145 driver — a port of the `SI1145` PyPI package the Python
//! node uses (itself a port of Adafruit's Arduino library): the same reset
//! sequence, the same UV calibration coefficients, the same channel list
//! and ADC settings, and the same autonomous measurement mode, so the
//! counts this node reports match the Python node's.
//!
//! The chip computes the UV index itself from the visible/IR photodiodes
//! and reports it multiplied by 100; visible and IR are raw counts (the
//! SI1145 is not lux-calibrated) that sit at a dark baseline of roughly
//! 250-260 rather than 0. The scaling is platform-neutral (and unit
//! tested); only the I2C access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Convert the chip's raw UV register value to a UV index: the SI1145
/// reports the index multiplied by 100.
pub fn uv_index(raw: u16) -> f64 {
    f64::from(raw) / 100.0
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;
    use std::thread::sleep;
    use std::time::Duration;

    // -- SI1145 register map ---------------------------------------------

    const ADDRESS: u16 = 0x60;

    const REG_PART_ID: u8 = 0x00; // reads 0x45 on an SI1145
    const REG_INT_CFG: u8 = 0x03;
    const REG_IRQ_EN: u8 = 0x04;
    const REG_IRQ_MODE1: u8 = 0x05;
    const REG_IRQ_MODE2: u8 = 0x06;
    const REG_HW_KEY: u8 = 0x07;
    const REG_MEAS_RATE0: u8 = 0x08;
    const REG_MEAS_RATE1: u8 = 0x09;
    const REG_PS_LED21: u8 = 0x0F;
    const REG_UCOEFF0: u8 = 0x13;
    const REG_PARAM_WR: u8 = 0x17;
    const REG_COMMAND: u8 = 0x18;
    const REG_IRQ_STAT: u8 = 0x21;
    const REG_ALS_VIS_DATA: u8 = 0x22; // 16-bit little-endian, like the two below
    const REG_ALS_IR_DATA: u8 = 0x24;
    const REG_UV_INDEX: u8 = 0x2C;
    const REG_PARAM_RD: u8 = 0x2E;

    const PART_ID: u8 = 0x45;

    // Commands.
    const CMD_RESET: u8 = 0x01;
    const CMD_PARAM_SET: u8 = 0xA0;
    const CMD_PSALS_AUTO: u8 = 0x0F;

    // Parameter RAM addresses.
    const PARAM_CHLIST: u8 = 0x01;
    const PARAM_PSLED12SEL: u8 = 0x02;
    const PARAM_PS1_ADC_MUX: u8 = 0x07;
    const PARAM_PS_ADC_COUNTER: u8 = 0x0A;
    const PARAM_PS_ADC_GAIN: u8 = 0x0B;
    const PARAM_PS_ADC_MISC: u8 = 0x0C;
    const PARAM_ALS_IR_ADC_MUX: u8 = 0x0E;
    const PARAM_ALS_VIS_ADC_COUNTER: u8 = 0x10;
    const PARAM_ALS_VIS_ADC_GAIN: u8 = 0x11;
    const PARAM_ALS_VIS_ADC_MISC: u8 = 0x12;
    const PARAM_ALS_IR_ADC_COUNTER: u8 = 0x1D;
    const PARAM_ALS_IR_ADC_GAIN: u8 = 0x1E;
    const PARAM_ALS_IR_ADC_MISC: u8 = 0x1F;

    // Parameter values.
    const CHLIST_EN_UV: u8 = 0x80;
    const CHLIST_EN_ALS_IR: u8 = 0x20;
    const CHLIST_EN_ALS_VIS: u8 = 0x10;
    const CHLIST_EN_PS1: u8 = 0x01;
    const INT_CFG_INT_OE: u8 = 0x01;
    const IRQ_EN_ALS_EVERY_SAMPLE: u8 = 0x01;
    const PSLED12SEL_PS1LED1: u8 = 0x01;
    const ADC_COUNTER_511CLK: u8 = 0x70;
    const ADC_MUX_SMALL_IR: u8 = 0x00;
    const ADC_MUX_LARGE_IR: u8 = 0x03;
    const PS_ADC_MISC_RANGE: u8 = 0x20;
    const PS_ADC_MISC_PS_MODE: u8 = 0x04;
    const ALS_VIS_ADC_MISC_VIS_RANGE: u8 = 0x20;
    const ALS_IR_ADC_MISC_RANGE: u8 = 0x20;

    pub struct Si1145 {
        dev: I2CDevice,
    }

    impl Si1145 {
        /// Open the sensor on /dev/i2c-<bus> at 0x60, reset it and start
        /// the autonomous measurement loop.
        pub fn new(bus: u8) -> Result<Self, String> {
            let dev = I2CDevice::open(bus, ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let mut sensor = Self { dev };
            // The Python library does not check the part ID; doing so turns
            // a missing or wrong chip into a clear error instead of
            // nonsense counts.
            let part_id = sensor
                .dev
                .read_reg(REG_PART_ID)
                .map_err(|e| e.to_string())?;
            if part_id != PART_ID {
                return Err(format!(
                    "No SI1145 at {ADDRESS:#04x} (part ID {part_id:#04x}, \
                     expected {PART_ID:#04x})"
                ));
            }
            sensor.reset()?;
            sensor.load_calibration()?;
            Ok(sensor)
        }

        fn write(&mut self, reg: u8, value: u8) -> Result<(), String> {
            self.dev.write_reg(reg, value).map_err(|e| e.to_string())
        }

        /// Stop any running measurement, clear the interrupt state and
        /// unlock the chip with the hardware key from the datasheet.
        fn reset(&mut self) -> Result<(), String> {
            for (reg, value) in [
                (REG_MEAS_RATE0, 0x00),
                (REG_MEAS_RATE1, 0x00),
                (REG_IRQ_EN, 0x00),
                (REG_IRQ_MODE1, 0x00),
                (REG_IRQ_MODE2, 0x00),
                (REG_INT_CFG, 0x00),
                (REG_IRQ_STAT, 0xFF),
                (REG_COMMAND, CMD_RESET),
            ] {
                self.write(reg, value)?;
            }
            sleep(Duration::from_millis(10));
            self.write(REG_HW_KEY, 0x17)?;
            sleep(Duration::from_millis(10));
            Ok(())
        }

        /// Write one byte of parameter RAM, which is only reachable through
        /// the command register.
        fn write_param(&mut self, param: u8, value: u8) -> Result<(), String> {
            self.write(REG_PARAM_WR, value)?;
            self.write(REG_COMMAND, param | CMD_PARAM_SET)?;
            // Read back, as the library does.
            self.dev.read_reg(REG_PARAM_RD).map_err(|e| e.to_string())?;
            Ok(())
        }

        /// Write the UV coefficients, enable the UV, visible, IR and
        /// proximity channels, configure the ADCs for the fastest clock
        /// with 511-clock measurements, and start autonomous sampling every
        /// 8 ms.
        fn load_calibration(&mut self) -> Result<(), String> {
            // UV index coefficients from the datasheet's default calibration.
            for (offset, value) in [0x29u8, 0x89, 0x02, 0x00].iter().enumerate() {
                self.write(REG_UCOEFF0 + offset as u8, *value)?;
            }
            self.write_param(
                PARAM_CHLIST,
                CHLIST_EN_UV | CHLIST_EN_ALS_IR | CHLIST_EN_ALS_VIS | CHLIST_EN_PS1,
            )?;
            // Interrupt on every ALS sample (wired out on some breakouts;
            // the node polls the data registers regardless).
            self.write(REG_INT_CFG, INT_CFG_INT_OE)?;
            self.write(REG_IRQ_EN, IRQ_EN_ALS_EVERY_SAMPLE)?;
            // Proximity: 20 mA on LED 1, high range, large-IR photodiode.
            self.write(REG_PS_LED21, 0x03)?;
            for (param, value) in [
                (PARAM_PS1_ADC_MUX, ADC_MUX_LARGE_IR),
                (PARAM_PSLED12SEL, PSLED12SEL_PS1LED1),
                (PARAM_PS_ADC_GAIN, 0),
                (PARAM_PS_ADC_COUNTER, ADC_COUNTER_511CLK),
                (PARAM_PS_ADC_MISC, PS_ADC_MISC_RANGE | PS_ADC_MISC_PS_MODE),
                // Ambient light: small-IR photodiode for IR, both in high range.
                (PARAM_ALS_IR_ADC_MUX, ADC_MUX_SMALL_IR),
                (PARAM_ALS_IR_ADC_GAIN, 0),
                (PARAM_ALS_IR_ADC_COUNTER, ADC_COUNTER_511CLK),
                (PARAM_ALS_IR_ADC_MISC, ALS_IR_ADC_MISC_RANGE),
                (PARAM_ALS_VIS_ADC_GAIN, 0),
                (PARAM_ALS_VIS_ADC_COUNTER, ADC_COUNTER_511CLK),
                (PARAM_ALS_VIS_ADC_MISC, ALS_VIS_ADC_MISC_VIS_RANGE),
            ] {
                self.write_param(param, value)?;
            }
            // 255 * 31.25 µs ≈ 8 ms between autonomous measurements.
            self.write(REG_MEAS_RATE0, 0xFF)?;
            self.write(REG_COMMAND, CMD_PSALS_AUTO)
        }

        /// The latest visible and IR counts and the UV index.
        pub fn read(&mut self) -> Result<(u16, u16, f64), String> {
            let visible = self
                .dev
                .read_word(REG_ALS_VIS_DATA)
                .map_err(|e| e.to_string())?;
            let ir = self
                .dev
                .read_word(REG_ALS_IR_DATA)
                .map_err(|e| e.to_string())?;
            let raw_uv = self
                .dev
                .read_word(REG_UV_INDEX)
                .map_err(|e| e.to_string())?;
            Ok((visible, ir, uv_index(raw_uv)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::round1;

    /// The chip reports the UV index multiplied by 100.
    #[test]
    fn uv_index_scaling() {
        for (raw, want) in [
            (0u16, 0.0),
            (50, 0.5),
            (123, 1.23),     // the payload rounds this to 1.2
            (800, 8.0),      // "very high" on the UV index scale
            (65535, 655.35), // full scale; far beyond any real UV index
        ] {
            let got = uv_index(raw);
            assert!(
                (got - want).abs() < 1e-9,
                "uv_index({raw}) = {got}, want {want}"
            );
        }
    }

    /// The REST payload reports one decimal, like the Python node.
    #[test]
    fn uv_index_payload_rounding() {
        assert_eq!(round1(uv_index(123)), 1.2);
        assert_eq!(round1(uv_index(475)), 4.8);
    }
}
