//! Compact BME280 driver, ported from the RPi.bme280 Python driver
//! (https://github.com/rm-hull/bme280, MIT) — the library the Python node
//! uses — so the readings match it: double-precision compensation formulas
//! from Appendix A (8.1) of the BME280 datasheet, forced mode with x1
//! oversampling. The compensation math is platform-neutral (and unit
//! tested); only the I2C access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[derive(Default, Clone)]
pub struct Calibration {
    pub t1: f64, pub t2: f64, pub t3: f64,
    pub p1: f64, pub p2: f64, pub p3: f64, pub p4: f64, pub p5: f64,
    pub p6: f64, pub p7: f64, pub p8: f64, pub p9: f64,
    pub h1: f64, pub h2: f64, pub h3: f64, pub h4: f64, pub h5: f64, pub h6: f64,
}

/// Parse the calibration EEPROM blocks; mirrors RPi.bme280's
/// load_calibration_params: 16-bit words are little-endian; H4/H5 share a
/// nibble, assembled from *signed* byte reads exactly as the reference does.
pub fn parse_calibration(c1: &[u8], h1: u8, c2: &[u8]) -> Calibration {
    let u16le = |b: &[u8], i: usize| f64::from(u16::from(b[i]) | u16::from(b[i + 1]) << 8);
    let s16le = |b: &[u8], i: usize| f64::from((u16::from(b[i]) | u16::from(b[i + 1]) << 8) as i16);
    let s8 = |v: u8| i32::from(v as i8);

    let (e4, e5, e6) = (s8(c2[3]), s8(c2[4]), s8(c2[5]));
    Calibration {
        t1: u16le(c1, 0), t2: s16le(c1, 2), t3: s16le(c1, 4),
        p1: u16le(c1, 6), p2: s16le(c1, 8), p3: s16le(c1, 10),
        p4: s16le(c1, 12), p5: s16le(c1, 14), p6: s16le(c1, 16),
        p7: s16le(c1, 18), p8: s16le(c1, 20), p9: s16le(c1, 22),
        h1: f64::from(h1),
        h2: s16le(c2, 0),
        h3: f64::from(c2[2] as i8),
        h4: f64::from((e4 << 4) | (e5 & 0x0f)),
        h5: f64::from(((e5 >> 4) & 0x0f) | (e6 << 4)),
        h6: f64::from(c2[6] as i8),
    }
}

fn t_fine(c: &Calibration, t: f64) -> f64 {
    let v1 = (t / 16384.0 - c.t1 / 1024.0) * c.t2;
    let d = t / 131072.0 - c.t1 / 8192.0;
    v1 + d * d * c.t3
}

/// Raw ADC values -> (temperature °C, pressure hPa, humidity %RH), with
/// the double-precision formulas as transcribed in RPi.bme280.
pub fn compensate(c: &Calibration, raw_t: i64, raw_p: i64, raw_h: i64) -> (f64, f64, f64) {
    let fine = t_fine(c, raw_t as f64);
    let temperature = fine / 5120.0;

    // Humidity.
    let mut res = fine - 76800.0;
    res = (raw_h as f64 - (c.h4 * 64.0 + c.h5 / 16384.0 * res))
        * (c.h2 / 65536.0 * (1.0 + c.h6 / 67108864.0 * res * (1.0 + c.h3 / 67108864.0 * res)));
    res *= 1.0 - c.h1 * res / 524288.0;
    let humidity = res.clamp(0.0, 100.0);

    // Pressure (Pa, then hPa).
    let mut v1 = fine / 2.0 - 64000.0;
    let mut v2 = v1 * v1 * c.p6 / 32768.0;
    v2 += v1 * c.p5 * 2.0;
    v2 = v2 / 4.0 + c.p4 * 65536.0;
    v1 = (c.p3 * v1 * v1 / 524288.0 + c.p2 * v1) / 524288.0;
    v1 = (1.0 + v1 / 32768.0) * c.p1;
    if v1 == 0.0 {
        return (temperature, 0.0, humidity);
    }
    let mut p = 1048576.0 - raw_p as f64;
    p = (p - v2 / 4096.0) * 6250.0 / v1;
    v1 = c.p9 * p * p / 2147483648.0;
    v2 = p * c.p8 / 32768.0;
    p += (v1 + v2 + c.p7) / 16.0;
    (temperature, p / 100.0, humidity)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;
    use std::time::Duration;

    const CALIBRATION_1: u8 = 0x88; // dig_T*, dig_P* (24 bytes)
    const DIG_H1: u8 = 0xa1;
    const CALIBRATION_2: u8 = 0xe1; // dig_H2..dig_H6 (7 bytes)
    const CTRL_HUM: u8 = 0xf2;
    const CTRL_MEAS: u8 = 0xf4;
    const DATA: u8 = 0xf7; // press msb..hum lsb (8 bytes)

    const OVERSAMPLING: u8 = 1; // x1, the RPi.bme280 default
    const FORCED_MODE: u8 = 1;

    pub struct Bme280 {
        dev: I2CDevice,
        cal: Calibration,
    }

    impl Bme280 {
        /// Open the sensor on /dev/i2c-<bus> at 0x76 and load calibration.
        pub fn new(bus: u8) -> Result<Self, String> {
            let mut dev = I2CDevice::open(bus, 0x76)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let c1 = dev.read_regs(CALIBRATION_1, 24).map_err(|e| e.to_string())?;
            let h1 = dev.read_reg(DIG_H1).map_err(|e| e.to_string())?;
            let c2 = dev.read_regs(CALIBRATION_2, 7).map_err(|e| e.to_string())?;
            Ok(Self {
                dev,
                cal: parse_calibration(&c1, h1, &c2),
            })
        }

        /// One forced x1-oversampling measurement:
        /// (temperature °C, pressure hPa, humidity %RH).
        pub fn read(&mut self) -> Result<(f64, f64, f64), String> {
            self.dev
                .write_reg(CTRL_HUM, OVERSAMPLING)
                .map_err(|e| e.to_string())?;
            self.dev
                .write_reg(CTRL_MEAS, OVERSAMPLING << 5 | OVERSAMPLING << 2 | FORCED_MODE)
                .map_err(|e| e.to_string())?;
            // RPi.bme280's __calc_delay for x1/x1/x1, rounded up.
            std::thread::sleep(Duration::from_millis(12));

            let b = self.dev.read_regs(DATA, 8).map_err(|e| e.to_string())?;
            let raw_p = (i64::from(b[0]) << 16 | i64::from(b[1]) << 8 | i64::from(b[2])) >> 4;
            let raw_t = (i64::from(b[3]) << 16 | i64::from(b[4]) << 8 | i64::from(b[5])) >> 4;
            let raw_h = i64::from(b[6]) << 8 | i64::from(b[7]);
            Ok(compensate(&self.cal, raw_t, raw_p, raw_h))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    // Golden values generated with the RPi.bme280 Python driver's
    // double-precision formulas over a synthetic calibration set.
    #[test]
    fn compensation_matches_reference_goldens() {
        let data = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/compensation_goldens.json"
        ))
        .expect("goldens");
        let g: Value = serde_json::from_str(&data).unwrap();
        let cal = &g["cal"];
        let c = Calibration {
            t1: cal["T1"].as_f64().unwrap(), t2: cal["T2"].as_f64().unwrap(), t3: cal["T3"].as_f64().unwrap(),
            p1: cal["P1"].as_f64().unwrap(), p2: cal["P2"].as_f64().unwrap(), p3: cal["P3"].as_f64().unwrap(),
            p4: cal["P4"].as_f64().unwrap(), p5: cal["P5"].as_f64().unwrap(), p6: cal["P6"].as_f64().unwrap(),
            p7: cal["P7"].as_f64().unwrap(), p8: cal["P8"].as_f64().unwrap(), p9: cal["P9"].as_f64().unwrap(),
            h1: cal["H1"].as_f64().unwrap(), h2: cal["H2"].as_f64().unwrap(), h3: cal["H3"].as_f64().unwrap(),
            h4: cal["H4"].as_f64().unwrap(), h5: cal["H5"].as_f64().unwrap(), h6: cal["H6"].as_f64().unwrap(),
        };
        for case in g["cases"].as_array().unwrap() {
            let (t, p, h) = compensate(
                &c,
                case["raw_t"].as_i64().unwrap(),
                case["raw_p"].as_i64().unwrap(),
                case["raw_h"].as_i64().unwrap(),
            );
            assert!((t - case["t"].as_f64().unwrap()).abs() < 1e-9, "temperature");
            assert!((p - case["p"].as_f64().unwrap()).abs() < 1e-9, "pressure");
            assert!((h - case["h"].as_f64().unwrap()).abs() < 1e-9, "humidity");
        }
    }

    // H4/H5 shared-nibble assembly from signed byte reads.
    #[test]
    fn calibration_h4_h5_signed_assembly() {
        let c1 = [0u8; 24];
        let c2 = [0x78, 0x01, 0x00, 0x11, 0xc8, 0x1e, 0x1e];
        let cal = parse_calibration(&c1, 75, &c2);
        assert_eq!(cal.h4, 280.0);
        assert_eq!(cal.h5, 492.0);
    }
}
