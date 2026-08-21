//! BMP085 / BMP180 barometer driver — a direct port of the integer
//! compensation algorithm written out in python/bmp085/sensor_node.py
//! (from the BMP085 datasheet, section 3.5), so all the nodes stay in
//! step. The pin-compatible BMP180 works unchanged. The compensation math
//! is platform-neutral (and unit tested); only the I2C access is
//! Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Standard sea-level pressure, in pascal. Altitude is relative to this,
/// so it moves with the weather as much as with the height.
pub const SEA_LEVEL_PA: f64 = 101325.0;

/// The eleven factory constants stored in the sensor's EEPROM. MB is
/// parsed for completeness but never used: the datasheet's compensation
/// algorithm has no term for it (the Python node reads it the same way).
#[derive(Clone)]
#[cfg_attr(target_os = "linux", allow(dead_code))]
pub struct Calibration {
    pub ac1: i64, pub ac2: i64, pub ac3: i64, pub ac4: i64, pub ac5: i64, pub ac6: i64,
    pub b1: i64, pub b2: i64, pub mb: i64, pub mc: i64, pub md: i64,
}

/// Parse the 22 calibration bytes. AC4, AC5 and AC6 are the only unsigned
/// words. The datasheet states no calibration word is ever 0x0000 or
/// 0xFFFF — exactly what a bus with nothing on it reads back — so that is
/// rejected here rather than compensated with.
pub fn parse_calibration(data: &[u8]) -> Result<Calibration, String> {
    if data.len() != 22 {
        return Err(format!("need 22 calibration bytes, got {}", data.len()));
    }
    let signedness = [true, true, true, false, false, false, true, true, true, true, true];
    let mut values = [0i64; 11];
    for (index, signed) in signedness.iter().enumerate() {
        let raw = u16::from(data[index * 2]) << 8 | u16::from(data[index * 2 + 1]);
        if raw == 0x0000 || raw == 0xFFFF {
            return Err(format!(
                "implausible calibration word {index} = 0x{raw:04X} (bad I2C read?)"
            ));
        }
        values[index] = if *signed { i64::from(raw as i16) } else { i64::from(raw) };
    }
    Ok(Calibration {
        ac1: values[0], ac2: values[1], ac3: values[2], ac4: values[3],
        ac5: values[4], ac6: values[5], b1: values[6], b2: values[7],
        mb: values[8], mc: values[9], md: values[10],
    })
}

/// Raw readings -> (temperature °C, pressure Pa) — a direct transcription
/// of the integer algorithm in the BMP085 datasheet. Rust's / truncates
/// toward zero like C's, so the one true division carries over as written.
pub fn compensate(c: &Calibration, raw_temperature: i64, raw_pressure: i64, oversampling: u8) -> (f64, i64) {
    // Temperature.
    let mut x1 = ((raw_temperature - c.ac6) * c.ac5) >> 15;
    let mut x2 = (c.mc * 2048) / (x1 + c.md);
    let b5 = x1 + x2;
    let temperature = (((b5 + 8) >> 4) as f64) / 10.0; // 0.1 °C steps

    // Pressure.
    let b6 = b5 - 4000;
    x1 = (c.b2 * ((b6 * b6) >> 12)) >> 11;
    x2 = (c.ac2 * b6) >> 11;
    let mut x3 = x1 + x2;
    let b3 = (((c.ac1 * 4 + x3) << oversampling) + 2) >> 2;
    x1 = (c.ac3 * b6) >> 13;
    x2 = (c.b1 * ((b6 * b6) >> 12)) >> 16;
    x3 = ((x1 + x2) + 2) >> 2;
    let b4 = (c.ac4 * (x3 + 32768)) >> 15;
    let b7 = (raw_pressure - b3) * (50000 >> oversampling);
    // B7 is unsigned 32-bit in the datasheet and can exceed 2^31, which is
    // why it is scaled before rather than after the division in that case.
    let mut pressure = if b7 < 0x80000000 { (b7 * 2) / b4 } else { (b7 / b4) * 2 };
    x1 = (pressure >> 8) * (pressure >> 8);
    x1 = (x1 * 3038) >> 16;
    x2 = (-7357 * pressure) >> 16;
    pressure += (x1 + x2 + 3791) >> 4;

    (temperature, pressure)
}

/// Altitude in metres from pressure, per the international barometric
/// formula the BMP085 datasheet quotes.
pub fn altitude_for(pressure_pa: f64) -> f64 {
    44330.0 * (1.0 - (pressure_pa / SEA_LEVEL_PA).powf(1.0 / 5.255))
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;
    use std::time::Duration;

    const I2C_ADDRESS: u16 = 0x77; // fixed — the BMP085 has no address pin

    const REG_CALIBRATION: u8 = 0xAA; // 22 bytes: AC1..AC6, B1, B2, MB, MC, MD
    const REG_CHIP_ID: u8 = 0xD0; // reads 0x55 on a BMP085 (and a BMP180)
    const REG_CONTROL: u8 = 0xF4;
    const REG_DATA: u8 = 0xF6;

    const CMD_READ_TEMPERATURE: u8 = 0x2E;
    const CMD_READ_PRESSURE: u8 = 0x34;

    const CHIP_ID: u8 = 0x55;

    // Conversion time per oversampling setting (datasheet table 3,
    // rounded up for margin).
    const CONVERSION_TIME_MS: [u64; 4] = [5, 8, 14, 26];

    pub struct Bmp085 {
        dev: I2CDevice,
        cal: Calibration,
        oversampling: u8,
    }

    impl Bmp085 {
        pub fn new(bus: u8, oversampling: u8) -> Result<Self, String> {
            if oversampling > 3 {
                return Err(format!("oversampling must be 0-3, got {oversampling}"));
            }
            let mut dev = I2CDevice::open(bus, I2C_ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let chip_id = dev.read_reg(REG_CHIP_ID).map_err(|e| e.to_string())?;
            if chip_id != CHIP_ID {
                return Err(format!(
                    "No BMP085 at address {I2C_ADDRESS:#04x}: chip id is \
                     {chip_id:#04x}, expected {CHIP_ID:#04x}."
                ));
            }
            let raw = dev.read_regs(REG_CALIBRATION, 22).map_err(|e| e.to_string())?;
            Ok(Self {
                dev,
                cal: parse_calibration(&raw)?,
                oversampling,
            })
        }

        fn read_raw_temperature(&mut self) -> std::io::Result<i64> {
            self.dev.write_reg(REG_CONTROL, CMD_READ_TEMPERATURE)?;
            std::thread::sleep(Duration::from_millis(CONVERSION_TIME_MS[0]));
            let d = self.dev.read_regs(REG_DATA, 2)?;
            Ok(i64::from(d[0]) << 8 | i64::from(d[1]))
        }

        fn read_raw_pressure(&mut self) -> std::io::Result<i64> {
            self.dev
                .write_reg(REG_CONTROL, CMD_READ_PRESSURE + (self.oversampling << 6))?;
            std::thread::sleep(Duration::from_millis(
                CONVERSION_TIME_MS[self.oversampling as usize],
            ));
            let d = self.dev.read_regs(REG_DATA, 3)?;
            let raw = i64::from(d[0]) << 16 | i64::from(d[1]) << 8 | i64::from(d[2]);
            Ok(raw >> (8 - self.oversampling))
        }

        /// One measurement: (temperature °C, pressure Pa). Temperature is
        /// read first, and every time — its B5 term feeds the pressure
        /// compensation, so a stale one skews the pressure as the chip
        /// warms.
        pub fn read(&mut self) -> Result<(f64, i64), String> {
            let raw_t = self.read_raw_temperature().map_err(|e| e.to_string())?;
            let raw_p = self.read_raw_pressure().map_err(|e| e.to_string())?;
            Ok(compensate(&self.cal, raw_t, raw_p, self.oversampling))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example from the BMP085 datasheet (section 3.5).
    #[test]
    fn datasheet_worked_example() {
        let c = Calibration {
            ac1: 408, ac2: -72, ac3: -14383, ac4: 32741, ac5: 32757, ac6: 23153,
            b1: 6190, b2: 4, mb: -32768, mc: -8711, md: 2868,
        };
        let (temperature, pressure) = compensate(&c, 27898, 23843, 0);
        assert_eq!(temperature, 15.0);
        assert_eq!(pressure, 69964);
    }

    #[test]
    fn altitude_at_sea_level() {
        assert!(altitude_for(SEA_LEVEL_PA).abs() < 1e-9);
        assert!((altitude_for(69964.0) - 3021.0).abs() < 30.0);
    }

    #[test]
    fn calibration_rejects_bus_garbage() {
        assert!(parse_calibration(&[0u8; 22]).is_err());
        assert!(parse_calibration(&[0xFFu8; 22]).is_err());
    }
}
