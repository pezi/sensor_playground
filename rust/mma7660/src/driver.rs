//! MMA7660 driver — the Grove 3-Axis Digital Accelerometer ±1.5g
//! (MMA7660FC), driven directly over I2C like the Python node's smbus2
//! access: each axis is a 6-bit two's-complement value with 21.33 counts
//! per g; bit 6 of a sample is the alert flag, meaning the register was
//! updated mid-read and must be read again. The axis decoding and the
//! roll/pitch math are platform-neutral (and unit tested); only the I2C
//! access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub const COUNTS_PER_G: f64 = 21.33;
const ALERT_BIT: u8 = 0x40;

/// One 6-bit two's-complement sample -> counts.
pub fn decode_axis(raw: u8) -> i32 {
    if raw > 31 {
        i32::from(raw) - 64
    } else {
        i32::from(raw)
    }
}

/// A 3-byte X/Y/Z block -> (x, y, z) in g, or None when any sample
/// carries the alert bit (updated mid-read — read again).
pub fn decode_axes(raw: &[u8]) -> Option<(f64, f64, f64)> {
    if raw.iter().any(|v| v & ALERT_BIT != 0) {
        return None;
    }
    let g = |v: u8| f64::from(decode_axis(v)) / COUNTS_PER_G;
    Some((g(raw[0]), g(raw[1]), g(raw[2])))
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

    const REG_X: u8 = 0x00;
    const REG_MODE: u8 = 0x07;
    const REG_SR: u8 = 0x08;
    const ADDRESS: u16 = 0x4c;

    pub struct Mma7660 {
        dev: I2CDevice,
    }

    impl Mma7660 {
        /// Open the sensor on /dev/i2c-<bus> at 0x4c and switch it active.
        pub fn new(bus: u8) -> Result<Self, String> {
            let mut dev = I2CDevice::open(bus, ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            // Standby to configure, 32 samples/s, then active mode.
            dev.write_reg(REG_MODE, 0x00).map_err(|e| e.to_string())?;
            dev.write_reg(REG_SR, 0x02).map_err(|e| e.to_string())?;
            dev.write_reg(REG_MODE, 0x01).map_err(|e| e.to_string())?;
            Ok(Self { dev })
        }

        /// Roll/pitch/g-force, re-reading while the alert bit is set.
        pub fn read(&mut self) -> Result<(f64, f64, f64), String> {
            for _ in 0..10 {
                let raw = self.dev.read_regs(REG_X, 3).map_err(|e| e.to_string())?;
                if let Some((x, y, z)) = decode_axes(&raw) {
                    return Ok(orientation(x, y, z));
                }
            }
            Err("MMA7660 kept reporting the alert bit".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(got: f64, want: f64) {
        assert!((got - want).abs() < 1e-9, "{got} != {want}");
    }

    // 6-bit two's complement: 0..31 positive, 32..63 wrap to -32..-1.
    #[test]
    fn six_bit_twos_complement_decoding() {
        assert_eq!(decode_axis(0), 0);
        assert_eq!(decode_axis(1), 1);
        assert_eq!(decode_axis(31), 31);
        assert_eq!(decode_axis(32), -32);
        assert_eq!(decode_axis(63), -1);
        assert_eq!(decode_axis(43), -21);
    }

    // Counts -> g with 21.33 counts per g.
    #[test]
    fn counts_to_g_conversion() {
        let (x, y, z) = decode_axes(&[21, 63, 32]).unwrap();
        approx(x, 21.0 / 21.33);
        approx(y, -1.0 / 21.33);
        approx(z, -32.0 / 21.33);
    }

    // The alert bit (0x40) invalidates the whole block.
    #[test]
    fn alert_bit_invalidates_block() {
        assert!(decode_axes(&[0x40, 0, 21]).is_none());
        assert!(decode_axes(&[0, 0x55, 21]).is_none());
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
