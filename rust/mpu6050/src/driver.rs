//! MPU6050 driver — the InvenSense 6-axis IMU, driven directly over I2C
//! like the Python node's smbus2 access: the chip wakes from sleep by
//! clearing PWR_MGMT_1, then a 14-byte burst read from ACCEL_XOUT_H
//! delivers the accelerometer (big-endian 16-bit words, 16384 LSB/g at
//! the default ±2 g range), temperature (raw/340 + 36.53 °C) and
//! gyroscope values (the gyroscope words are part of the burst but
//! unused, like the Python node). The decoding and the roll/pitch math
//! are platform-neutral (and unit tested); only the I2C access is
//! Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub const LSB_PER_G: f64 = 16384.0;

/// The big-endian two's-complement word at raw[i], raw[i + 1].
pub fn s16be(raw: &[u8], i: usize) -> i32 {
    i32::from(i16::from_be_bytes([raw[i], raw[i + 1]]))
}

/// The 14-byte ACCEL_XOUT_H burst -> (x, y, z, temperature): axis g
/// values plus the die temperature in °C.
pub fn decode_accel_temp(raw: &[u8]) -> (f64, f64, f64, f64) {
    let g = |i| f64::from(s16be(raw, i)) / LSB_PER_G;
    let temperature = f64::from(s16be(raw, 6)) / 340.0 + 36.53;
    (g(0), g(2), g(4), temperature)
}

/// Axis g values -> (roll °, pitch °, g-force): roll / pitch angles plus
/// the total acceleration magnitude.
pub fn orientation(x: f64, y: f64, z: f64) -> (f64, f64, f64) {
    let roll = y.atan2(z).to_degrees();
    let pitch = (-x).atan2((y * y + z * z).sqrt()).to_degrees();
    let gforce = (x * x + y * y + z * z).sqrt();
    (roll, pitch, gforce)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;

    const REG_PWR_MGMT_1: u8 = 0x6b;
    const REG_ACCEL_XOUT_H: u8 = 0x3b;
    const ADDRESS: u16 = 0x68;

    pub struct Mpu6050 {
        dev: I2CDevice,
    }

    impl Mpu6050 {
        /// Open the sensor on /dev/i2c-<bus> at 0x68 and wake it from
        /// sleep (the chip powers up sleeping).
        pub fn new(bus: u8) -> Result<Self, String> {
            let mut dev = I2CDevice::open(bus, ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            dev.write_reg(REG_PWR_MGMT_1, 0x00)
                .map_err(|e| e.to_string())?;
            Ok(Self { dev })
        }

        /// (temperature °C, roll °, pitch °, g-force) from one burst read.
        pub fn read(&mut self) -> Result<(f64, f64, f64, f64), String> {
            let raw = self
                .dev
                .read_regs(REG_ACCEL_XOUT_H, 14)
                .map_err(|e| e.to_string())?;
            let (x, y, z, temperature) = decode_accel_temp(&raw);
            let (roll, pitch, gforce) = orientation(x, y, z);
            Ok((temperature, roll, pitch, gforce))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-9, "{got} != {want}");
    }

    // Big-endian two's complement: 0x8000 wraps to -32768, 0xFFFF to -1.
    #[test]
    fn big_endian_sixteen_bit_decoding() {
        assert_eq!(s16be(&[0x00, 0x00], 0), 0);
        assert_eq!(s16be(&[0x00, 0x01], 0), 1);
        assert_eq!(s16be(&[0x7f, 0xff], 0), 32767);
        assert_eq!(s16be(&[0x80, 0x00], 0), -32768);
        assert_eq!(s16be(&[0xff, 0xff], 0), -1);
        assert_eq!(s16be(&[0xc0, 0x00], 0), -16384);
    }

    // Raw -> g with 16384 LSB per g, and raw/340 + 36.53 for the die
    // temperature; the trailing gyroscope words are ignored.
    #[test]
    fn raw_to_g_and_temperature_conversion() {
        let raw = [
            0x20, 0x00, // ax =   8192 -> 0.5 g
            0xc0, 0x00, // ay = -16384 -> -1.0 g
            0x40, 0x00, // az =  16384 -> 1.0 g
            0xf9, 0x5c, // temp = -1700 -> -5.0 + 36.53 = 31.53 °C
            0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, // gyroscope (unused)
        ];
        let (x, y, z, temperature) = decode_accel_temp(&raw);
        approx(x, 0.5);
        approx(y, -1.0);
        approx(z, 1.0);
        approx(temperature, 31.53);
    }

    #[test]
    fn roll_pitch_gforce_math() {
        // Flat board resting at 1 g: no roll, no pitch.
        let (roll, pitch, gforce) = orientation(0.0, 0.0, 1.0);
        approx(roll, 0.0);
        approx(pitch, 0.0);
        approx(gforce, 1.0);

        // 45° roll: y and z pull equally.
        let (roll, pitch, _) = orientation(0.0, 0.7, 0.7);
        approx(roll, 45.0);
        approx(pitch, 0.0);

        // Nose-down: gravity entirely on -x -> +90° pitch.
        let (_, pitch, gforce) = orientation(-1.0, 0.0, 0.0);
        approx(pitch, 90.0);
        approx(gforce, 1.0);
    }
}
