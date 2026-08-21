//! Compact TCS34725 driver — a port of the `adafruit_tcs34725` library the
//! Python node uses: the same sensor-ID check, the same 154 ms / 4x
//! profile, the same gamma-corrected RGB bytes, and the same DN40 lux and
//! colour temperature algorithm, so the values match the Python node.
//!
//! Every register access ORs the address with the command bit 0x80,
//! exactly as the Python library does — that is the byte sequence the
//! reference node puts on the wire, block reads included.
//!
//! The colour maths is platform-neutral (and unit tested); only the I2C
//! access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// DN40 device-specific values (DN40 Table 1, Appendix I).
const GLASS_ATTENUATION: f64 = 1.0;
const DEVICE_FACTOR: f64 = 310.0;
const R_COEF: f64 = 0.136;
const G_COEF: f64 = 1.0;
const B_COEF: f64 = -0.444;
const CT_COEF: f64 = 3810.0;
const CT_OFFSET: f64 = 1391.0;

/// The 0-255 colour bytes the app paints.
pub struct ColorBytes {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

/// Normalize the red, green and blue counts against the clear channel and
/// apply the library's 2.5 gamma correction. Complete darkness (clear == 0)
/// is black.
pub fn color_rgb_bytes(r: u16, g: u16, b: u16, clear: u16) -> ColorBytes {
    if clear == 0 {
        return ColorBytes {
            red: 0,
            green: 0,
            blue: 0,
        };
    }
    // Truncation at both steps, like the Python library's int() calls.
    let channel = |value: u16| -> u8 {
        let scaled = (f64::from(value) / f64::from(clear) * 256.0).trunc() / 255.0;
        (scaled.powf(2.5) * 255.0).trunc().min(255.0) as u8
    };
    ColorBytes {
        red: channel(r),
        green: channel(g),
        blue: channel(b),
    }
}

/// Convert raw R/G/B/C counts to illuminance and colour temperature with
/// the algorithm from Taos/AMS design note DN40. None when the clear
/// channel saturated: the sample says nothing about the colour then, so the
/// node reports no reading at all.
pub fn temperature_and_lux_dn40(
    r: u16,
    g: u16,
    b: u16,
    c: u16,
    integration_ms: f64,
    gain: f64,
) -> Option<(f64, f64)> {
    // Analog/digital saturation (DN40 3.5). The ATIME register holds
    // 256 - cycles, so cycles is what the count limit scales with.
    let cycles = (integration_ms / 2.4).round();
    let mut saturation = if cycles > 63.0 {
        65535.0
    } else {
        1024.0 * cycles
    };
    // Ripple saturation (DN40 3.7): below 150 ms the 50/60 Hz ripple of
    // mains-powered light eats into the usable range.
    if integration_ms < 150.0 {
        saturation -= saturation / 4.0;
    }
    if f64::from(c) >= saturation {
        return None;
    }

    let (red, green, blue, clear) = (f64::from(r), f64::from(g), f64::from(b), f64::from(c));

    // IR rejection (DN40 3.1): the excess of R+G+B over the clear channel
    // is infrared leaking into all three colour channels.
    let infrared = if red + green + blue > clear {
        (red + green + blue - clear) / 2.0
    } else {
        0.0
    };
    let (r2, g2, b2) = (red - infrared, green - infrared, blue - infrared);

    // Lux (DN40 3.2).
    let g1 = R_COEF * r2 + G_COEF * g2 + B_COEF * b2;
    let mut cpl = (integration_ms * gain) / (GLASS_ATTENUATION * DEVICE_FACTOR);
    if cpl == 0.0 {
        cpl = 0.001;
    }

    // Colour temperature (DN40 3.4).
    let r2 = if r2 == 0.0 { 0.001 } else { r2 };
    Some((g1 / cpl, CT_COEF * b2 / r2 + CT_OFFSET))
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;
    use std::thread::sleep;
    use std::time::{Duration, Instant};

    // -- TCS34725 register map -------------------------------------------

    const ADDRESS: u16 = 0x29;
    const COMMAND_BIT: u8 = 0x80; // every register access is OR'd with this

    const REG_ENABLE: u8 = 0x00;
    const REG_ATIME: u8 = 0x01;
    const REG_CONTROL: u8 = 0x0F;
    const REG_SENSOR_ID: u8 = 0x12;
    const REG_STATUS: u8 = 0x13;
    const REG_CDATA: u8 = 0x14; // C, R, G, B — eight consecutive bytes, LE words

    const ENABLE_AEN: u8 = 0x02; // ADC enable
    const ENABLE_PON: u8 = 0x01; // power on

    const STATUS_AVALID: u8 = 0x01; // a conversion has completed

    /// The gain register holds the *index* into this table.
    const GAINS: [f64; 4] = [1.0, 4.0, 16.0, 60.0];

    pub struct Tcs34725 {
        dev: I2CDevice,
        // The integration time and gain actually programmed, kept here so
        // the DN40 maths needs no extra register reads per measurement.
        integration_ms: f64,
        gain: f64,
    }

    impl Tcs34725 {
        /// Open the sensor on /dev/i2c-<bus> at 0x29, verify the sensor ID
        /// and program the same profile as the Python node: 154 ms
        /// integration at 4x gain, with the ADC left enabled.
        ///
        /// The library's own defaults (2.4 ms, 1x) collect almost no light
        /// — every channel reads 0 in normal room light and the readings
        /// collapse to constants — which is why both nodes override them.
        pub fn new(bus: u8) -> Result<Self, String> {
            let dev = I2CDevice::open(bus, ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let mut sensor = Self {
                dev,
                integration_ms: 0.0,
                gain: 0.0,
            };
            let sensor_id = sensor.read_u8(REG_SENSOR_ID)?;
            // The library accepts all three IDs the TCS3472 family reports.
            if !matches!(sensor_id, 0x44 | 0x10 | 0x4D) {
                return Err(format!(
                    "No TCS34725 at {ADDRESS:#04x} (sensor ID {sensor_id:#04x}, \
                     expected 0x44, 0x10 or 0x4d)"
                ));
            }
            sensor.set_integration_time(154.0)?;
            sensor.set_gain(4.0)?;
            sensor.activate()?;
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

        /// Program the integration time in milliseconds. The chip counts
        /// 2.4 ms cycles and ATIME holds 256 - cycles.
        fn set_integration_time(&mut self, milliseconds: f64) -> Result<(), String> {
            if !(2.4..=614.4).contains(&milliseconds) {
                return Err(format!(
                    "integration time must be between 2.4 and 614.4 ms, got {milliseconds}"
                ));
            }
            let cycles = (milliseconds / 2.4) as u32;
            self.integration_ms = f64::from(cycles) * 2.4;
            self.write_u8(REG_ATIME, (256 - cycles) as u8)
        }

        /// Program the analog gain; the register holds the index into GAINS.
        fn set_gain(&mut self, gain: f64) -> Result<(), String> {
            let Some(index) = GAINS.iter().position(|candidate| *candidate == gain) else {
                return Err(format!("gain must be one of 1, 4, 16, 60, got {gain}"));
            };
            self.gain = gain;
            self.write_u8(REG_CONTROL, index as u8)
        }

        /// Power the chip and enable the ADC, leaving it running — the
        /// Python node does the same instead of toggling it around every
        /// read.
        fn activate(&mut self) -> Result<(), String> {
            let enable = self.read_u8(REG_ENABLE)?;
            self.write_u8(REG_ENABLE, enable | ENABLE_PON)?;
            sleep(Duration::from_millis(3)); // the oscillator needs 2.4 ms
            self.write_u8(REG_ENABLE, enable | ENABLE_PON | ENABLE_AEN)
        }

        /// Wait for a completed conversion and return the raw 16-bit red,
        /// green, blue and clear counts.
        fn read_raw(&mut self) -> Result<(u16, u16, u16, u16), String> {
            // One integration period is the longest this can take; give it
            // a few so a conversion that started just before the call still
            // counts.
            let timeout = Duration::from_millis((3.0 * self.integration_ms + 50.0) as u64);
            let deadline = Instant::now() + timeout;
            while self.read_u8(REG_STATUS)? & STATUS_AVALID == 0 {
                if Instant::now() > deadline {
                    return Err(format!("no completed conversion after {timeout:?}"));
                }
                sleep(Duration::from_millis((self.integration_ms + 0.9) as u64));
            }
            let d = self
                .dev
                .read_regs(COMMAND_BIT | REG_CDATA, 8)
                .map_err(|e| e.to_string())?;
            let word = |lo: usize| u16::from(d[lo]) | u16::from(d[lo + 1]) << 8;
            // The block starts at the clear channel, then red, green, blue.
            Ok((word(2), word(4), word(6), word(0)))
        }

        /// One measurement: the colour bytes, the illuminance and the
        /// colour temperature, or None when the clear channel saturated.
        ///
        /// Unlike the Python node — which reads the sensor once per
        /// property and so three times per request — this derives all three
        /// from a single conversion, which also keeps them consistent with
        /// each other.
        pub fn read(&mut self) -> Result<Option<(ColorBytes, f64, f64)>, String> {
            let (r, g, b, c) = self.read_raw()?;
            let Some((lux, color_temperature)) =
                temperature_and_lux_dn40(r, g, b, c, self.integration_ms, self.gain)
            else {
                return Ok(None);
            };
            Ok(Some((color_rgb_bytes(r, g, b, c), lux, color_temperature)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The node's profile: 154 ms requested -> 64 cycles -> 153.6 ms, at 4x.
    const INTEGRATION_MS: f64 = 153.6;
    const GAIN: f64 = 4.0;

    /// The gamma-corrected RGB bytes must match the Python library's. The
    /// expected values come from running the reference implementation on
    /// the same raw counts.
    #[test]
    fn gamma_corrected_color_bytes() {
        for (r, g, b, c, want) in [
            (1000u16, 1200u16, 900u16, 3500u16, (11u8, 17u8, 8u8)),
            (5000, 4200, 3000, 12000, (28, 18, 8)),
            (300, 400, 350, 900, (16, 33, 23)),
            // Clear at zero is complete darkness: black, not a divide by zero.
            (0, 0, 0, 0, (0, 0, 0)),
        ] {
            let got = color_rgb_bytes(r, g, b, c);
            assert_eq!(
                (got.red, got.green, got.blue),
                want,
                "color_rgb_bytes({r},{g},{b},{c})"
            );
        }
    }

    /// A fully lit channel saturates at 255 rather than overflowing past it.
    #[test]
    fn color_bytes_clamp_to_a_byte() {
        let got = color_rgb_bytes(65535, 65535, 65535, 65535);
        assert_eq!((got.red, got.green, got.blue), (255, 255, 255));
    }

    #[test]
    fn dn40_lux_and_color_temperature() {
        for (r, g, b, c, want_lux, want_ct) in [
            (
                1000u16,
                1200u16,
                900u16,
                3500u16,
                472.46744791666663,
                4820.0,
            ),
            (
                5000,
                4200,
                3000,
                12000,
                1755.2539062499998,
                3645.8979591836733,
            ),
            (300, 400, 350, 900, 117.81412760416667, 6047.666666666667),
            // All channels dark: no light and the bare CT offset.
            (0, 0, 0, 0, 0.0, 1391.0),
        ] {
            let (lux, ct) =
                temperature_and_lux_dn40(r, g, b, c, INTEGRATION_MS, GAIN).expect("not saturated");
            assert!(
                (lux - want_lux).abs() < 1e-9,
                "lux({r},{g},{b},{c}) = {lux}"
            );
            assert!((ct - want_ct).abs() < 1e-9, "ct({r},{g},{b},{c}) = {ct}");
        }
    }

    /// A saturated clear channel says nothing about the colour, so the
    /// sample is rejected instead of reported as a very bright reading.
    #[test]
    fn dn40_rejects_saturation() {
        assert!(
            temperature_and_lux_dn40(60000, 60000, 60000, 65535, INTEGRATION_MS, GAIN).is_none()
        );
        // Below 150 ms the saturation limit drops by a quarter (DN40 3.7):
        // at 100.8 ms the limit is 1024*42*0.75 = 32256 counts.
        assert!(temperature_and_lux_dn40(9000, 9000, 9000, 32256, 100.8, GAIN).is_none());
        let (lux, _) =
            temperature_and_lux_dn40(1000, 1200, 900, 3500, 100.8, GAIN).expect("not saturated");
        assert!(
            (lux - 719.9503968253969).abs() < 1e-9,
            "lux at 100.8 ms = {lux}"
        );
    }
}
