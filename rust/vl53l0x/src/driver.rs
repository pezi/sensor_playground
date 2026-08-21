//! Compact VL53L0X driver, ported from the Adafruit CircuitPython driver
//! adafruit_vl53l0x (https://github.com/adafruit/Adafruit_CircuitPython_VL53L0X,
//! MIT) — the library the Python node uses, itself adapted from the Pololu
//! vl53l0x-arduino code: the full init register sequence (reference SPAD
//! selection, tuning settings, interrupt config, timing-budget preservation,
//! VHV/phase ref calibration), then single-shot ranging reads in
//! millimeters. The timeout encoding and timing-budget math are
//! platform-neutral (and unit tested); only the I2C access is Linux-only.
//!
//! Like the Python node (which leaves the driver's io_timeout_s at 0), the
//! wait loops poll without a timeout — an unresponsive sensor surfaces as
//! an I2C error, not a hang, because every poll is a bus transaction.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

// -- Platform-neutral math (mirrors the Adafruit driver's helpers) -----------

/// Decode a timeout register value; format: "(LSByte * 2^MSByte) + 1".
pub fn decode_timeout(val: u16) -> f64 {
    f64::from(val & 0xFF) * 2.0_f64.powi(i32::from((val & 0xFF00) >> 8)) + 1.0
}

/// Encode a timeout in MCLKs into the register format
/// "(LSByte * 2^MSByte) + 1".
pub fn encode_timeout(timeout_mclks: f64) -> u16 {
    let mclks = (timeout_mclks as u32) & 0xFFFF;
    let mut ls_byte;
    let mut ms_byte = 0u32;
    if mclks > 0 {
        ls_byte = mclks - 1;
        while ls_byte > 255 {
            ls_byte >>= 1;
            ms_byte += 1;
        }
        ((ms_byte << 8 | ls_byte & 0xFF) & 0xFFFF) as u16
    } else {
        0
    }
}

/// Convert a timeout in macro clocks to microseconds for the given VCSEL
/// period (integer-floor arithmetic like the reference driver).
pub fn timeout_mclks_to_us(timeout_period_mclks: f64, vcsel_period_pclks: u32) -> f64 {
    let macro_period_ns = (2304 * vcsel_period_pclks * 1655 + 500) / 1000;
    ((timeout_period_mclks * f64::from(macro_period_ns) + f64::from(macro_period_ns / 2)) / 1000.0)
        .floor()
}

/// Convert a timeout in microseconds to macro clocks for the given VCSEL
/// period.
pub fn timeout_us_to_mclks(timeout_period_us: f64, vcsel_period_pclks: u32) -> f64 {
    let macro_period_ns = (2304 * vcsel_period_pclks * 1655 + 500) / 1000;
    ((timeout_period_us * 1000.0 + f64::from(macro_period_ns / 2)) / f64::from(macro_period_ns))
        .floor()
}

/// Decode a VCSEL period register value into PCLKs.
pub fn decode_vcsel_period(reg_val: u8) -> u32 {
    u32::from((u16::from(reg_val) + 1) & 0xFF) << 1
}

pub struct SequenceEnables {
    pub tcc: bool,
    pub dss: bool,
    pub msrc: bool,
    pub pre_range: bool,
    pub final_range: bool,
}

/// Decode SYSTEM_SEQUENCE_CONFIG (based on VL53L0X_GetSequenceStepEnables
/// from the ST API).
pub fn sequence_step_enables(sequence_config: u8) -> SequenceEnables {
    SequenceEnables {
        tcc: sequence_config >> 4 & 0x1 > 0,
        dss: sequence_config >> 3 & 0x1 > 0,
        msrc: sequence_config >> 2 & 0x1 > 0,
        pre_range: sequence_config >> 6 & 0x1 > 0,
        final_range: sequence_config >> 7 & 0x1 > 0,
    }
}

/// Clear every SPAD enable bit outside the good reference window (the
/// first 12 bits are aperture SPADs) and past `spad_count` enabled ones,
/// returning how many stayed enabled.
pub fn mask_ref_spad_map(ref_spad_map: &mut [u8; 6], spad_count: u8, spad_is_aperture: bool) -> u8 {
    let first_spad_to_enable = if spad_is_aperture { 12 } else { 0 };
    let mut spads_enabled = 0u8;
    for i in 0..48usize {
        if i < first_spad_to_enable || spads_enabled == spad_count {
            // This bit is lower than the first one that should be enabled,
            // or (reference_spad_count) bits have already been enabled, so
            // zero this bit.
            ref_spad_map[i / 8] &= !(1 << (i % 8));
        } else if ref_spad_map[i / 8] >> (i % 8) & 0x1 > 0 {
            spads_enabled += 1;
        }
    }
    spads_enabled
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;

    const I2C_SLAVE: libc::c_ulong = 0x0703; // linux/i2c-dev.h

    const VL53L0X_ADDR: libc::c_ulong = 0x29; // the fixed power-on address (41)

    // Registers (the subset the driver uses, names from the Adafruit driver).
    const SYSRANGE_START: u8 = 0x00;
    const SYSTEM_SEQUENCE_CONFIG: u8 = 0x01;
    const SYSTEM_INTERRUPT_CONFIG_GPIO: u8 = 0x0A;
    const SYSTEM_INTERRUPT_CLEAR: u8 = 0x0B;
    const RESULT_INTERRUPT_STATUS: u8 = 0x13;
    const RESULT_RANGE_STATUS: u8 = 0x14;
    const FINAL_RANGE_CONFIG_MIN_COUNT_RATE_RTN_LIMIT: u8 = 0x44;
    const MSRC_CONFIG_TIMEOUT_MACROP: u8 = 0x46;
    const DYNAMIC_SPAD_NUM_REQUESTED_REF_SPAD: u8 = 0x4E;
    const DYNAMIC_SPAD_REF_EN_START_OFFSET: u8 = 0x4F;
    const PRE_RANGE_CONFIG_VCSEL_PERIOD: u8 = 0x50;
    const PRE_RANGE_CONFIG_TIMEOUT_MACROP_HI: u8 = 0x51;
    const MSRC_CONFIG_CONTROL: u8 = 0x60;
    const FINAL_RANGE_CONFIG_VCSEL_PERIOD: u8 = 0x70;
    const FINAL_RANGE_CONFIG_TIMEOUT_MACROP_HI: u8 = 0x71;
    const GPIO_HV_MUX_ACTIVE_HIGH: u8 = 0x84;
    const GLOBAL_CONFIG_SPAD_ENABLES_REF_0: u8 = 0xB0;
    const GLOBAL_CONFIG_REF_EN_START_SELECT: u8 = 0xB6;

    /// The block of undocumented "default tuning settings" every VL53L0X
    /// driver writes after the SPAD map (ST API load_tuning_settings).
    const TUNING: [(u8, u8); 80] = [
        (0xFF, 0x01), (0x00, 0x00), (0xFF, 0x00), (0x09, 0x00), (0x10, 0x00),
        (0x11, 0x00), (0x24, 0x01), (0x25, 0xFF), (0x75, 0x00), (0xFF, 0x01),
        (0x4E, 0x2C), (0x48, 0x00), (0x30, 0x20), (0xFF, 0x00), (0x30, 0x09),
        (0x54, 0x00), (0x31, 0x04), (0x32, 0x03), (0x40, 0x83), (0x46, 0x25),
        (0x60, 0x00), (0x27, 0x00), (0x50, 0x06), (0x51, 0x00), (0x52, 0x96),
        (0x56, 0x08), (0x57, 0x30), (0x61, 0x00), (0x62, 0x00), (0x64, 0x00),
        (0x65, 0x00), (0x66, 0xA0), (0xFF, 0x01), (0x22, 0x32), (0x47, 0x14),
        (0x49, 0xFF), (0x4A, 0x00), (0xFF, 0x00), (0x7A, 0x0A), (0x7B, 0x00),
        (0x78, 0x21), (0xFF, 0x01), (0x23, 0x34), (0x42, 0x00), (0x44, 0xFF),
        (0x45, 0x26), (0x46, 0x05), (0x40, 0x40), (0x0E, 0x06), (0x20, 0x1A),
        (0x43, 0x40), (0xFF, 0x00), (0x34, 0x03), (0x35, 0x44), (0xFF, 0x01),
        (0x31, 0x04), (0x4B, 0x09), (0x4C, 0x05), (0x4D, 0x04), (0xFF, 0x00),
        (0x44, 0x00), (0x45, 0x20), (0x47, 0x08), (0x48, 0x28), (0x67, 0x00),
        (0x70, 0x04), (0x71, 0x01), (0x72, 0xFE), (0x76, 0x00), (0x77, 0x00),
        (0xFF, 0x01), (0x0D, 0x01), (0xFF, 0x00), (0x80, 0x01), (0x01, 0xF8),
        (0xFF, 0x01), (0x8E, 0x01), (0x00, 0x01), (0xFF, 0x00), (0x80, 0x00),
    ];

    pub struct Vl53l0x {
        f: File,
        stop_variable: u8,
    }

    impl Vl53l0x {
        /// Open the sensor on /dev/i2c-<bus> at 0x29 and run the full init
        /// sequence, like the Python node's driver.
        pub fn new(bus: u8) -> Result<Self, String> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, VL53L0X_ADDR) };
            if rc != 0 {
                return Err(format!(
                    "I2C_SLAVE ioctl for 0x29: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let mut s = Self {
                f,
                stop_variable: 0,
            };
            s.init()?;
            Ok(s)
        }

        fn read_u8(&mut self, reg: u8) -> Result<u8, String> {
            self.f.write_all(&[reg]).map_err(|e| e.to_string())?;
            let mut buf = [0u8; 1];
            self.f.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(buf[0])
        }

        fn read_u16(&mut self, reg: u8) -> Result<u16, String> {
            self.f.write_all(&[reg]).map_err(|e| e.to_string())?;
            let mut buf = [0u8; 2];
            self.f.read_exact(&mut buf).map_err(|e| e.to_string())?;
            Ok(u16::from(buf[0]) << 8 | u16::from(buf[1])) // big-endian
        }

        fn write_u8(&mut self, reg: u8, val: u8) -> Result<(), String> {
            self.f.write_all(&[reg, val]).map_err(|e| e.to_string())
        }

        fn write_u16(&mut self, reg: u8, val: u16) -> Result<(), String> {
            self.f
                .write_all(&[reg, (val >> 8) as u8, val as u8]) // big-endian
                .map_err(|e| e.to_string())
        }

        fn write_pairs(&mut self, pairs: &[(u8, u8)]) -> Result<(), String> {
            for &(reg, val) in pairs {
                self.write_u8(reg, val)?;
            }
            Ok(())
        }

        /// Mirrors the Adafruit driver's __init__ (data init, static init,
        /// SPAD management, tuning settings, interrupt config, timing
        /// budget, ref calibration).
        fn init(&mut self) -> Result<(), String> {
            // Check identification registers for expected values
            // (datasheet 3.2).
            if self.read_u8(0xC0)? != 0xEE
                || self.read_u8(0xC1)? != 0xAA
                || self.read_u8(0xC2)? != 0x10
            {
                return Err("failed to find expected ID register values, check wiring".into());
            }
            // Set I2C standard mode.
            self.write_pairs(&[(0x88, 0x00), (0x80, 0x01), (0xFF, 0x01), (0x00, 0x00)])?;
            self.stop_variable = self.read_u8(0x91)?;
            self.write_pairs(&[(0x00, 0x01), (0xFF, 0x00), (0x80, 0x00)])?;
            // Disable SIGNAL_RATE_MSRC (bit 1) and SIGNAL_RATE_PRE_RANGE
            // (bit 4) limit checks.
            let config_control = self.read_u8(MSRC_CONFIG_CONTROL)? | 0x12;
            self.write_u8(MSRC_CONFIG_CONTROL, config_control)?;
            // Set final range signal rate limit to 0.25 MCPS (million
            // counts per second), as 16-bit 9.7 fixed point.
            self.write_u16(
                FINAL_RANGE_CONFIG_MIN_COUNT_RATE_RTN_LIMIT,
                (0.25 * f64::from(1 << 7)) as u16,
            )?;
            self.write_u8(SYSTEM_SEQUENCE_CONFIG, 0xFF)?;

            let (spad_count, spad_is_aperture) = self.get_spad_info()?;
            // The SPAD map (RefGoodSpadMap) is read by
            // VL53L0X_get_info_from_device() in the API, but the same data
            // is more easily readable from GLOBAL_CONFIG_SPAD_ENABLES_REF_0
            // through _6, so read it from there.
            self.f
                .write_all(&[GLOBAL_CONFIG_SPAD_ENABLES_REF_0])
                .map_err(|e| e.to_string())?;
            let mut ref_spad_map = [0u8; 6];
            self.f
                .read_exact(&mut ref_spad_map)
                .map_err(|e| e.to_string())?;
            self.write_pairs(&[
                (0xFF, 0x01),
                (DYNAMIC_SPAD_REF_EN_START_OFFSET, 0x00),
                (DYNAMIC_SPAD_NUM_REQUESTED_REF_SPAD, 0x2C),
                (0xFF, 0x00),
                (GLOBAL_CONFIG_REF_EN_START_SELECT, 0xB4),
            ])?;
            mask_ref_spad_map(&mut ref_spad_map, spad_count, spad_is_aperture);
            let mut msg = [0u8; 7];
            msg[0] = GLOBAL_CONFIG_SPAD_ENABLES_REF_0;
            msg[1..].copy_from_slice(&ref_spad_map);
            self.f.write_all(&msg).map_err(|e| e.to_string())?;

            self.write_pairs(&TUNING)?;
            self.write_u8(SYSTEM_INTERRUPT_CONFIG_GPIO, 0x04)?;
            let gpio_hv_mux = self.read_u8(GPIO_HV_MUX_ACTIVE_HIGH)?;
            self.write_u8(GPIO_HV_MUX_ACTIVE_HIGH, gpio_hv_mux & !0x10)?; // active low
            self.write_u8(SYSTEM_INTERRUPT_CLEAR, 0x01)?;

            let budget_us = self.get_measurement_timing_budget()?;
            self.write_u8(SYSTEM_SEQUENCE_CONFIG, 0xE8)?;
            self.set_measurement_timing_budget(budget_us)?;
            self.write_u8(SYSTEM_SEQUENCE_CONFIG, 0x01)?;
            self.perform_single_ref_calibration(0x40)?;
            self.write_u8(SYSTEM_SEQUENCE_CONFIG, 0x02)?;
            self.perform_single_ref_calibration(0x00)?;
            // "restore the previous Sequence Config"
            self.write_u8(SYSTEM_SEQUENCE_CONFIG, 0xE8)
        }

        /// Reference SPAD count and type (is_aperture), based on the
        /// Pololu vl53l0x-arduino code.
        fn get_spad_info(&mut self) -> Result<(u8, bool), String> {
            self.write_pairs(&[(0x80, 0x01), (0xFF, 0x01), (0x00, 0x00), (0xFF, 0x06)])?;
            let v = self.read_u8(0x83)?;
            self.write_u8(0x83, v | 0x04)?;
            self.write_pairs(&[
                (0xFF, 0x07),
                (0x81, 0x01),
                (0x80, 0x01),
                (0x94, 0x6B),
                (0x83, 0x00),
            ])?;
            while self.read_u8(0x83)? == 0x00 {
                // wait for the device (no timeout, like the Python node)
            }
            self.write_u8(0x83, 0x01)?;
            let tmp = self.read_u8(0x92)?;
            let count = tmp & 0x7F;
            let is_aperture = tmp >> 7 & 0x01 == 1;
            self.write_pairs(&[(0x81, 0x00), (0xFF, 0x06)])?;
            let v = self.read_u8(0x83)?;
            self.write_u8(0x83, v & !0x04)?;
            self.write_pairs(&[(0xFF, 0x01), (0x00, 0x01), (0xFF, 0x00), (0x80, 0x00)])?;
            Ok((count, is_aperture))
        }

        /// Based on VL53L0X_perform_single_ref_calibration() from the ST
        /// API.
        fn perform_single_ref_calibration(&mut self, vhv_init_byte: u8) -> Result<(), String> {
            self.write_u8(SYSRANGE_START, 0x01 | vhv_init_byte)?;
            while self.read_u8(RESULT_INTERRUPT_STATUS)? & 0x07 == 0 {
                // wait for the device (no timeout, like the Python node)
            }
            self.write_u8(SYSTEM_INTERRUPT_CLEAR, 0x01)?;
            self.write_u8(SYSRANGE_START, 0x00)
        }

        fn get_vcsel_pulse_period(&mut self, reg: u8) -> Result<u32, String> {
            Ok(decode_vcsel_period(self.read_u8(reg)?))
        }

        /// Based on get_sequence_step_timeout() from the ST API, modified
        /// like the Pololu code. Returns (msrc_dss_tcc_us, pre_range_us,
        /// final_range_us, final_range_vcsel_period_pclks, pre_range_mclks).
        fn get_sequence_step_timeouts(
            &mut self,
            pre_range: bool,
        ) -> Result<(f64, f64, f64, u32, f64), String> {
            let pre_range_vcsel_period_pclks =
                self.get_vcsel_pulse_period(PRE_RANGE_CONFIG_VCSEL_PERIOD)?;
            let msrc_dss_tcc_mclks =
                f64::from((u16::from(self.read_u8(MSRC_CONFIG_TIMEOUT_MACROP)?) + 1) & 0xFF);
            let msrc_dss_tcc_us =
                timeout_mclks_to_us(msrc_dss_tcc_mclks, pre_range_vcsel_period_pclks);
            let pre_range_mclks =
                decode_timeout(self.read_u16(PRE_RANGE_CONFIG_TIMEOUT_MACROP_HI)?);
            let pre_range_us = timeout_mclks_to_us(pre_range_mclks, pre_range_vcsel_period_pclks);
            let final_range_vcsel_period_pclks =
                self.get_vcsel_pulse_period(FINAL_RANGE_CONFIG_VCSEL_PERIOD)?;
            let mut final_range_mclks =
                decode_timeout(self.read_u16(FINAL_RANGE_CONFIG_TIMEOUT_MACROP_HI)?);
            if pre_range {
                final_range_mclks -= pre_range_mclks;
            }
            let final_range_us =
                timeout_mclks_to_us(final_range_mclks, final_range_vcsel_period_pclks);
            Ok((
                msrc_dss_tcc_us,
                pre_range_us,
                final_range_us,
                final_range_vcsel_period_pclks,
                pre_range_mclks,
            ))
        }

        /// The measurement timing budget in microseconds.
        fn get_measurement_timing_budget(&mut self) -> Result<f64, String> {
            let mut budget_us = f64::from(1910 + 960); // start + end overhead
            let enables = sequence_step_enables(self.read_u8(SYSTEM_SEQUENCE_CONFIG)?);
            let (msrc_dss_tcc_us, pre_range_us, final_range_us, _, _) =
                self.get_sequence_step_timeouts(enables.pre_range)?;
            if enables.tcc {
                budget_us += msrc_dss_tcc_us + 590.0;
            }
            if enables.dss {
                budget_us += 2.0 * (msrc_dss_tcc_us + 690.0);
            } else if enables.msrc {
                budget_us += msrc_dss_tcc_us + 660.0;
            }
            if enables.pre_range {
                budget_us += pre_range_us + 660.0;
            }
            if enables.final_range {
                budget_us += final_range_us + 550.0;
            }
            Ok(budget_us)
        }

        /// Apply a measurement timing budget in microseconds by giving the
        /// final range step whatever time the other enabled steps leave
        /// over.
        fn set_measurement_timing_budget(&mut self, budget_us: f64) -> Result<(), String> {
            if budget_us < 20000.0 {
                return Err(format!(
                    "timing budget {budget_us} us is below the 20000 us minimum"
                ));
            }
            let mut used_budget_us = f64::from(1320 + 960); // start (diff from get) + end
            let enables = sequence_step_enables(self.read_u8(SYSTEM_SEQUENCE_CONFIG)?);
            let (msrc_dss_tcc_us, pre_range_us, _, final_range_vcsel_period_pclks, pre_range_mclks) =
                self.get_sequence_step_timeouts(enables.pre_range)?;
            if enables.tcc {
                used_budget_us += msrc_dss_tcc_us + 590.0;
            }
            if enables.dss {
                used_budget_us += 2.0 * (msrc_dss_tcc_us + 690.0);
            } else if enables.msrc {
                used_budget_us += msrc_dss_tcc_us + 660.0;
            }
            if enables.pre_range {
                used_budget_us += pre_range_us + 660.0;
            }
            if enables.final_range {
                used_budget_us += 550.0;
                // "Note that the final range timeout is determined by the
                // timing budget and the sum of all other timeouts within
                // the sequence. If there is no room for the final range
                // timeout, then an error will be set. Otherwise the
                // remaining time will be applied to the final range."
                if used_budget_us > budget_us {
                    return Err("requested timeout too big".into());
                }
                let mut final_range_timeout_mclks = timeout_us_to_mclks(
                    budget_us - used_budget_us,
                    final_range_vcsel_period_pclks,
                );
                if enables.pre_range {
                    final_range_timeout_mclks += pre_range_mclks;
                }
                self.write_u16(
                    FINAL_RANGE_CONFIG_TIMEOUT_MACROP_HI,
                    encode_timeout(final_range_timeout_mclks),
                )?;
            }
            Ok(())
        }

        /// One single-shot range measurement in millimeters (adapted from
        /// readRangeSingleMillimeters in the Pololu code; assumes the
        /// default linearity corrective gain of 1000 and no fractional
        /// ranging).
        pub fn read_range(&mut self) -> Result<u16, String> {
            let stop_variable = self.stop_variable;
            self.write_pairs(&[
                (0x80, 0x01),
                (0xFF, 0x01),
                (0x00, 0x00),
                (0x91, stop_variable),
                (0x00, 0x01),
                (0xFF, 0x00),
                (0x80, 0x00),
                (SYSRANGE_START, 0x01),
            ])?;
            while self.read_u8(SYSRANGE_START)? & 0x01 > 0 {
                // wait for the device (no timeout, like the Python node)
            }
            while self.read_u8(RESULT_INTERRUPT_STATUS)? & 0x07 == 0 {
                // wait for the device (no timeout, like the Python node)
            }
            let range_mm = self.read_u16(RESULT_RANGE_STATUS + 10)?;
            self.write_u8(SYSTEM_INTERRUPT_CLEAR, 0x01)?;
            Ok(range_mm)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Timeout register decoding/encoding ("(LSByte * 2^MSByte) + 1");
    // 0x0096 and 0x01FE are the pre/final range timeouts the tuning
    // settings write.
    #[test]
    fn timeout_coding() {
        for (reg, mclks) in [(0x0096u16, 151.0), (0x01FE, 509.0), (0x0180, 257.0)] {
            assert_eq!(decode_timeout(reg), mclks, "decode(0x{reg:04X})");
            assert_eq!(encode_timeout(mclks), reg, "encode({mclks})");
        }
        assert_eq!(encode_timeout(0.0), 0);
    }

    // MCLK <-> microsecond conversion for the tuning-default VCSEL periods
    // (pre-range register 0x06 -> 14 PCLKs, final-range 0x04 -> 10 PCLKs).
    #[test]
    fn timeout_conversions() {
        assert_eq!(decode_vcsel_period(0x06), 14);
        assert_eq!(decode_vcsel_period(0x04), 10);
        assert_eq!(timeout_mclks_to_us(38.0, 14), 2055.0); // MSRC (register 0x25 -> 38 MCLKs)
        assert_eq!(timeout_mclks_to_us(151.0, 14), 8087.0); // pre range
        assert_eq!(timeout_mclks_to_us(509.0 - 151.0, 10), 13669.0); // final minus pre range
        assert_eq!(timeout_us_to_mclks(14259.0, 10), 374.0);
    }

    #[test]
    fn sequence_enables() {
        // 0xE8 is the sequence config the driver leaves active.
        let e = sequence_step_enables(0xE8);
        assert!(!e.tcc && e.dss && !e.msrc && e.pre_range && e.final_range);
        let e = sequence_step_enables(0xFF);
        assert!(e.tcc && e.dss && e.msrc && e.pre_range && e.final_range);
    }

    // The timing budget the getter computes from the tuning-default
    // registers, and the final range timeout the setter re-encodes for the
    // same budget — golden values generated with the adafruit_vl53l0x
    // reference driver's math.
    #[test]
    fn timing_budget_math() {
        let msrc_dss_tcc_us = timeout_mclks_to_us(38.0, 14);
        let pre_range_us = timeout_mclks_to_us(151.0, 14);
        let final_range_us = timeout_mclks_to_us(509.0 - 151.0, 10);

        // Getter with sequence config 0xE8 (dss + pre range + final range).
        let budget_us = f64::from(1910 + 960)
            + 2.0 * (msrc_dss_tcc_us + 690.0)
            + (pre_range_us + 660.0)
            + (final_range_us + 550.0);
        assert_eq!(budget_us, 31326.0);

        // Setter with the same budget.
        let used_budget_us =
            f64::from(1320 + 960) + 2.0 * (msrc_dss_tcc_us + 690.0) + (pre_range_us + 660.0) + 550.0;
        let final_range_timeout_mclks = timeout_us_to_mclks(budget_us - used_budget_us, 10) + 151.0;
        assert_eq!(encode_timeout(final_range_timeout_mclks), 0x0283);
    }

    #[test]
    fn ref_spad_map_masking() {
        // Aperture SPADs start at bit 12; the first 12 bits are cleared.
        let mut map = [0xFF; 6];
        assert_eq!(mask_ref_spad_map(&mut map, 5, true), 5);
        assert_eq!(map, [0x00, 0xF0, 0x01, 0x00, 0x00, 0x00]);

        let mut map = [0xFF; 6];
        assert_eq!(mask_ref_spad_map(&mut map, 3, false), 3);
        assert_eq!(map, [0x07, 0x00, 0x00, 0x00, 0x00, 0x00]);

        // Holes in the good SPAD map are skipped, not counted.
        let mut map = [0x00, 0xCC, 0x00, 0x00, 0x00, 0x00];
        assert_eq!(mask_ref_spad_map(&mut map, 2, true), 2);
        assert_eq!(map, [0x00, 0xC0, 0x00, 0x00, 0x00, 0x00]);

        // Asking for more SPADs than exist enables all remaining ones.
        let mut map = [0xFF; 6];
        assert_eq!(mask_ref_spad_map(&mut map, 48, true), 36);
        assert_eq!(map, [0x00, 0xF0, 0xFF, 0xFF, 0xFF, 0xFF]);
    }
}
