//! Drivers for the Grove IMU 10DOF board: the MPU9250 accelerometer +
//! AK8963 magnetometer, ported from the mpu9250-jmdev Python driver (the
//! library the Python node uses — bypass mode, 8 g / 16-bit full scale,
//! 100 Hz continuous magnetometer), and the BMP280 barometer with the
//! Bosch datasheet integer compensation, ported from the Python node's
//! own BMP280 class — so the readings match the Python node. The
//! scaling, the derived angles and the compensation are
//! platform-neutral (and unit tested); only the I2C access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// ACCEL_SCALE_MODIFIER_8G, the accelerometer full scale the Python node
/// configures (AFS_8G).
pub const ACCEL_SCALE: f64 = 8.0 / 32768.0;
/// MAGNOMETER_SCALE_MODIFIER_BIT_16, the magnetometer full scale the
/// Python node configures (AK8963_BIT_16).
pub const MAG_SCALE: f64 = 4912.0 / 32760.0;

/// One set of derived readings for the live payload.
pub struct Readings {
    pub temperature: f64,
    pub pressure: f64,
    pub roll: f64,
    pub pitch: f64,
    pub heading: f64,
    pub gforce: f64,
}

/// Scale a big-endian 6-byte accelerometer block to g at the 8 g full
/// scale.
pub fn convert_accel(data: &[u8]) -> (f64, f64, f64) {
    let s16 = |i: usize| f64::from(i16::from_be_bytes([data[i], data[i + 1]]));
    (
        s16(0) * ACCEL_SCALE,
        s16(2) * ACCEL_SCALE,
        s16(4) * ACCEL_SCALE,
    )
}

/// Scale a little-endian 7-byte magnetometer block (HXL..ST2) to µT at
/// the 16-bit full scale, applying the factory sensitivity. A set ST2
/// overflow bit yields zeros, like the reference driver.
pub fn convert_mag(data: &[u8], mag_cal: &[f64; 3]) -> (f64, f64, f64) {
    if data[6] & 0x08 == 0x08 {
        return (0.0, 0.0, 0.0); // magnetic sensor overflow
    }
    let s16 = |i: usize| f64::from(i16::from_le_bytes([data[i], data[i + 1]]));
    (
        s16(0) * MAG_SCALE * mag_cal[0],
        s16(2) * MAG_SCALE * mag_cal[1],
        s16(4) * MAG_SCALE * mag_cal[2],
    )
}

/// Reduce the MPU9250 axes to the node's derived readings: roll/pitch
/// from the accelerometer, compass heading from the raw magnetometer
/// (not tilt-compensated) and the total acceleration magnitude
/// (g-force).
pub fn angles_from(ax: f64, ay: f64, az: f64, mx: f64, my: f64) -> (f64, f64, f64, f64) {
    let roll = ay.atan2(az).to_degrees();
    let pitch = (-ax).atan2((ay * ay + az * az).sqrt()).to_degrees();
    let mut heading = my.atan2(mx).to_degrees() % 360.0;
    if heading < 0.0 {
        heading += 360.0; // normalize into [0, 360) like Python's %
    }
    let gforce = (ax * ax + ay * ay + az * az).sqrt();
    (roll, pitch, heading, gforce)
}

/// The BMP280 calibration EEPROM (registers 0x88..0x9F).
pub struct Bmp280Calibration {
    pub t1: i128,
    pub t2: i128,
    pub t3: i128,
    pub p1: i128,
    pub p2: i128,
    pub p3: i128,
    pub p4: i128,
    pub p5: i128,
    pub p6: i128,
    pub p7: i128,
    pub p8: i128,
    pub p9: i128,
}

/// Parse the little-endian dig_T*/dig_P* calibration words (T1/P1
/// unsigned, the rest signed).
pub fn parse_bmp280_calibration(calib: &[u8]) -> Bmp280Calibration {
    let u16 = |i: usize| i128::from(u16::from_le_bytes([calib[i], calib[i + 1]]));
    let s16 = |i: usize| i128::from(i16::from_le_bytes([calib[i], calib[i + 1]]));
    Bmp280Calibration {
        t1: u16(0),
        t2: s16(2),
        t3: s16(4),
        p1: u16(6),
        p2: s16(8),
        p3: s16(10),
        p4: s16(12),
        p5: s16(14),
        p6: s16(16),
        p7: s16(18),
        p8: s16(20),
        p9: s16(22),
    }
}

/// Python's `//` operator: the compensation's division rounds toward
/// negative infinity, Rust's `/` toward zero.
fn floor_div(a: i128, b: i128) -> i128 {
    let q = a / b;
    if a % b != 0 && (a < 0) != (b < 0) {
        q - 1
    } else {
        q
    }
}

/// Raw ADC values -> (temperature °C, pressure hPa) with the Bosch
/// datasheet integer algorithms (32-bit temperature, 64-bit pressure),
/// exactly as transcribed in the Python node. The math runs in i128 so
/// the intermediates behave like Python's unbounded integers.
pub fn compensate_bmp280(c: &Bmp280Calibration, adc_t: i64, adc_p: i64) -> (f64, f64) {
    let adc_t = i128::from(adc_t);
    let adc_p = i128::from(adc_p);

    // Temperature compensation (datasheet 32-bit integer algorithm).
    let mut var1 = (((adc_t >> 3) - (c.t1 << 1)) * c.t2) >> 11;
    let mut var2 = (((((adc_t >> 4) - c.t1) * ((adc_t >> 4) - c.t1)) >> 12) * c.t3) >> 14;
    let t_fine = var1 + var2;
    let temperature = ((t_fine * 5 + 128) >> 8) as f64 / 100.0;

    // Pressure compensation (datasheet 64-bit integer algorithm).
    var1 = t_fine - 128000;
    var2 = var1 * var1 * c.p6;
    var2 += (var1 * c.p5) << 17;
    var2 += c.p4 << 35;
    var1 = ((var1 * var1 * c.p3) >> 8) + ((var1 * c.p2) << 12);
    var1 = (((1_i128 << 47) + var1) * c.p1) >> 33;
    if var1 == 0 {
        return (temperature, 0.0); // avoid division by zero
    }
    let mut p = 1_048_576 - adc_p;
    p = floor_div(((p << 31) - var2) * 3125, var1);
    var1 = (c.p9 * (p >> 13) * (p >> 13)) >> 25;
    var2 = (c.p8 * p) >> 19;
    p = ((p + var1 + var2) >> 8) + (c.p7 << 4);

    // p is in Q24.8 Pa; convert to hPa.
    (temperature, (p as f64 / 256.0) / 100.0)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;
    use std::thread::sleep;
    use std::time::Duration;

    const MPU_ADDRESS: u16 = 0x68; // MPU9250 (MPU9050_ADDRESS_68)
    const AK_ADDRESS: u16 = 0x0c; // AK8963 magnetometer, visible in bypass mode
    const BMP_ADDRESS: u16 = 0x77; // BMP280 barometer

    // MPU9250 registers.
    const MPU_SMPLRT_DIV: u8 = 0x19;
    const MPU_CONFIG: u8 = 0x1a;
    const MPU_GYRO_CONFIG: u8 = 0x1b;
    const MPU_ACCEL_CONFIG: u8 = 0x1c;
    const MPU_ACCEL_CONFIG_2: u8 = 0x1d;
    const MPU_INT_PIN_CFG: u8 = 0x37;
    const MPU_ACCEL_OUT: u8 = 0x3b;
    const MPU_USER_CTRL: u8 = 0x6a;
    const MPU_PWR_MGMT_1: u8 = 0x6b;

    // AK8963 registers.
    const AK_MAGNET_OUT: u8 = 0x03; // HXL..HZH + ST2 (7 bytes)
    const AK_CNTL1: u8 = 0x0a;
    const AK_ASAX: u8 = 0x10; // factory sensitivity (FuseROM, 3 bytes)

    // Full-scale selections the Python node configures.
    const GFS_1000: u8 = 0x02; // gyro 1000 dps
    const AFS_8G: u8 = 0x02; // accel 8 g
    const AK_BIT_16: u8 = 0x01; // magnetometer 16-bit output
    const AK_MODE_C100HZ: u8 = 0x06; // continuous 100 Hz

    const BMP_CHIP_ID: u8 = 0x58; // value of the id register (0xD0) for the BMP280

    /// The MPU9250 accelerometer plus its AK8963 magnetometer.
    pub struct Mpu9250 {
        mpu: I2CDevice,
        ak: I2CDevice,
        mag_cal: [f64; 3], // AK8963 factory sensitivity adjustment
    }

    impl Mpu9250 {
        /// Open the MPU9250 and its AK8963 on /dev/i2c-<bus> and run the
        /// mpu9250-jmdev configuration sequence (retried up to three
        /// times, like the reference driver).
        pub fn new(bus: u8) -> Result<Self, String> {
            let mpu = I2CDevice::open(bus, MPU_ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let ak = I2CDevice::open(bus, AK_ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let mut dev = Self {
                mpu,
                ak,
                mag_cal: [1.0; 3],
            };
            let mut last = String::new();
            for _ in 0..3 {
                match dev.configure() {
                    Ok(()) => return Ok(dev),
                    Err(err) => last = err,
                }
            }
            Err(format!("configuring MPU9250: {last}"))
        }

        fn configure(&mut self) -> Result<(), String> {
            // MPU6500 core (configureMPU6500, no slave).
            let steps: [(u8, u8, u64); 9] = [
                (MPU_PWR_MGMT_1, 0x00, 100),           // sleep off
                (MPU_PWR_MGMT_1, 0x01, 100),           // auto select clock source
                (MPU_CONFIG, 0x00, 0),                 // DLPF_CFG
                (MPU_SMPLRT_DIV, 0x00, 0),             // sample rate divider
                (MPU_GYRO_CONFIG, GFS_1000 << 3, 0),   // gyro full scale select
                (MPU_ACCEL_CONFIG, AFS_8G << 3, 0),    // accel full scale select
                (MPU_ACCEL_CONFIG_2, 0x00, 0),         // A_DLPFCFG
                (MPU_INT_PIN_CFG, 0x02, 100),          // BYPASS_EN enable
                (MPU_USER_CTRL, 0x00, 100),            // disable master
            ];
            for (reg, val, wait_ms) in steps {
                self.mpu.write_reg(reg, val).map_err(|e| e.to_string())?;
                if wait_ms > 0 {
                    sleep(Duration::from_millis(wait_ms));
                }
            }

            // AK8963 magnetometer (configureAK8963): read the factory
            // sensitivity coefficients from FuseROM, then 16-bit
            // continuous 100 Hz mode.
            self.write_ak(0x00)?; // power down
            self.write_ak(0x0f)?; // FuseROM access mode
            let asa = self
                .ak
                .read_regs(AK_ASAX, 3)
                .map_err(|e| e.to_string())?;
            self.write_ak(0x00)?; // power down
            self.write_ak(AK_BIT_16 << 4 | AK_MODE_C100HZ)?; // scale + continuous mode
            for i in 0..3 {
                self.mag_cal[i] = (f64::from(asa[i]) - 128.0) / 256.0 + 1.0;
            }
            Ok(())
        }

        fn write_ak(&mut self, val: u8) -> Result<(), String> {
            self.ak.write_reg(AK_CNTL1, val).map_err(|e| e.to_string())?;
            sleep(Duration::from_millis(100));
            Ok(())
        }

        /// The acceleration in g; a failed I2C read yields zeros, like
        /// the reference driver's getDataError.
        pub fn read_accel(&mut self) -> (f64, f64, f64) {
            match self.mpu.read_regs(MPU_ACCEL_OUT, 6) {
                Ok(data) => convert_accel(&data),
                Err(_) => (0.0, 0.0, 0.0),
            }
        }

        /// The magnetic field in µT; a failed read yields zeros.
        pub fn read_mag(&mut self) -> (f64, f64, f64) {
            match self.ak.read_regs(AK_MAGNET_OUT, 7) {
                Ok(data) => convert_mag(&data, &self.mag_cal),
                Err(_) => (0.0, 0.0, 0.0),
            }
        }
    }

    /// The BMP280 barometer, driven with the Bosch datasheet algorithm
    /// like the Python node's own minimal driver. The Grove IMU 10DOF
    /// v2.0 (2016) replaced the original BMP180 with a BMP280; the two
    /// chips share the I2C address but use different registers and a
    /// different compensation algorithm.
    pub struct Bmp280 {
        dev: I2CDevice,
        cal: Bmp280Calibration,
    }

    impl Bmp280 {
        /// Open the barometer on /dev/i2c-<bus> at 0x77, check the chip
        /// id and start normal-mode x1 sampling.
        pub fn new(bus: u8) -> Result<Self, String> {
            let mut dev = I2CDevice::open(bus, BMP_ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let chip_id = dev.read_reg(0xd0).map_err(|e| e.to_string())?;
            if chip_id != BMP_CHIP_ID {
                return Err(format!(
                    "BMP280 not found at 0x{BMP_ADDRESS:02X} (id register returned 0x{chip_id:02X})"
                ));
            }
            let cal = parse_bmp280_calibration(&dev.read_regs(0x88, 24).map_err(|e| e.to_string())?);
            // ctrl_meas: temperature x1, pressure x1, normal mode.
            dev.write_reg(0xf4, 0x27).map_err(|e| e.to_string())?;
            // config: 1000 ms standby, filter off.
            dev.write_reg(0xf5, 0xa0).map_err(|e| e.to_string())?;
            sleep(Duration::from_millis(50));
            Ok(Self { dev, cal })
        }

        /// (temperature °C, pressure hPa).
        pub fn read(&mut self) -> Result<(f64, f64), String> {
            let raw = self.dev.read_regs(0xf7, 6).map_err(|e| e.to_string())?;
            let adc_p = i64::from(raw[0]) << 12 | i64::from(raw[1]) << 4 | i64::from(raw[2]) >> 4;
            let adc_t = i64::from(raw[3]) << 12 | i64::from(raw[4]) << 4 | i64::from(raw[5]) >> 4;
            Ok(compensate_bmp280(&self.cal, adc_t, adc_p))
        }
    }

    /// The whole board: roll/pitch/heading/g-force plus temperature and
    /// pressure.
    pub struct Imu10dof {
        mpu: Mpu9250,
        bmp: Bmp280,
    }

    impl Imu10dof {
        pub fn new(bus: u8) -> Result<Self, String> {
            Ok(Self {
                mpu: Mpu9250::new(bus)?,
                bmp: Bmp280::new(bus)?,
            })
        }

        pub fn read(&mut self) -> Result<Readings, String> {
            let (ax, ay, az) = self.mpu.read_accel();
            let (mx, my, _) = self.mpu.read_mag();
            let (roll, pitch, heading, gforce) = angles_from(ax, ay, az, mx, my);
            let (temperature, pressure) = self.bmp.read()?;
            Ok(Readings {
                temperature,
                pressure,
                roll,
                pitch,
                heading,
                gforce,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn approx(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-9, "{got} != {want}");
    }

    // Big-endian raw counts at the 8 g full scale: 16384 -> 4 g,
    // -8192 -> -2 g, 8192 -> 2 g.
    #[test]
    fn accel_conversion_at_8g() {
        let (x, y, z) = convert_accel(&[0x40, 0x00, 0xe0, 0x00, 0x20, 0x00]);
        approx(x, 4.0);
        approx(y, -2.0);
        approx(z, 2.0);
    }

    // Little-endian raw counts with the 4912/32760 µT-per-count scale:
    // 3276 -> 491.2 µT, -3276 -> -491.2 µT, 32760 -> 4912 µT — each
    // multiplied by its factory sensitivity coefficient.
    #[test]
    fn mag_conversion_at_16_bit() {
        let (x, y, z) = convert_mag(
            &[0xcc, 0x0c, 0x34, 0xf3, 0xf8, 0x7f, 0x00],
            &[1.0, 1.0, 0.5],
        );
        approx(x, 491.2);
        approx(y, -491.2);
        approx(z, 2456.0);
    }

    // A set ST2 overflow bit (0x08) discards the sample, like the
    // mpu9250-jmdev reference driver.
    #[test]
    fn mag_overflow_yields_zeros() {
        let mag = convert_mag(
            &[0xcc, 0x0c, 0x34, 0xf3, 0xf8, 0x7f, 0x08],
            &[1.0, 1.0, 1.0],
        );
        assert_eq!(mag, (0.0, 0.0, 0.0));
    }

    #[test]
    fn roll_pitch_heading_gforce_math() {
        // A level board pointing magnetic north-east: no roll/pitch,
        // 1 g, heading 45°.
        let (roll, pitch, heading, gforce) = angles_from(0.0, 0.0, 1.0, 10.0, 10.0);
        approx(roll, 0.0);
        approx(pitch, 0.0);
        approx(heading, 45.0);
        approx(gforce, 1.0);

        // Negative atan2 results normalize into [0, 360) like Python's %.
        let (_, _, heading, _) = angles_from(0.0, 0.0, 1.0, 10.0, -10.0);
        approx(heading, 315.0);

        // 45° roll: y and z pull equally.
        let (roll, ..) = angles_from(0.0, 0.7, 0.7, 1.0, 0.0);
        approx(roll, 45.0);

        // Nose-down: gravity entirely on -x -> +90° pitch.
        let (_, pitch, ..) = angles_from(-1.0, 0.0, 0.0, 1.0, 0.0);
        approx(pitch, 90.0);
    }

    // Golden values generated with the Python node's own BMP280 class
    // (the Bosch datasheet integer algorithm over a synthetic
    // calibration set).
    #[test]
    fn bmp280_compensation_matches_python_goldens() {
        let data = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/testdata/bmp280_goldens.json"
        ))
        .expect("goldens");
        let g: Value = serde_json::from_str(&data).unwrap();
        let cal = &g["cal"];
        let n = |key: &str| i128::from(cal[key].as_i64().unwrap());
        let c = Bmp280Calibration {
            t1: n("t1"), t2: n("t2"), t3: n("t3"),
            p1: n("p1"), p2: n("p2"), p3: n("p3"),
            p4: n("p4"), p5: n("p5"), p6: n("p6"),
            p7: n("p7"), p8: n("p8"), p9: n("p9"),
        };
        for case in g["cases"].as_array().unwrap() {
            let (t, p) = compensate_bmp280(
                &c,
                case["adc_t"].as_i64().unwrap(),
                case["adc_p"].as_i64().unwrap(),
            );
            approx(t, case["t"].as_f64().unwrap());
            approx(p, case["p"].as_f64().unwrap());
        }
    }
}
