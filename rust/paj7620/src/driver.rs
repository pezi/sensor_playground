//! PAJ7620U2 gesture driver, ported from Seeed's grove.py
//! grove_gesture_sensor.py (MIT) — the library the Python node uses — so
//! the detected gestures match it: the same wake-up retry, the same init
//! register table (both banks) and the same two-phase flag decoding with
//! the 0.8 s entry / 1.0 s quit waits around the combined
//! forward/backward gestures. The flag decoding and the gesture-code
//! mapping are platform-neutral (and unit tested); only the I2C access is
//! Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::time::Duration;

pub const ADDRESS: u16 = 0x73; // I2C address (equals the device ID, as in grove.py)
const REG_BANK_SEL: u8 = 0xEF; // register bank select
const REG_FLAG_0: u8 = 0x43; // gesture detection flags, bank 0
const REG_FLAG_1: u8 = 0x44; // wave flag, bank 0

/// grove.py's GES_ENTRY_TIME / GES_QUIT_TIME: after a directional flag
/// the driver waits and re-reads to catch a combined forward/backward,
/// and backs off after a forward/backward so the hand can leave.
pub const GES_ENTRY_TIME: Duration = Duration::from_millis(800);
pub const GES_QUIT_TIME: Duration = Duration::from_millis(1000);

const WAKE_ATTEMPTS: usize = 3;

// Gesture detection flags (register 0x43; the wave flag lives in 0x44).
const GES_RIGHT_FLAG: u8 = 1 << 0;
const GES_LEFT_FLAG: u8 = 1 << 1;
const GES_UP_FLAG: u8 = 1 << 2;
const GES_DOWN_FLAG: u8 = 1 << 3;
const GES_FORWARD_FLAG: u8 = 1 << 4;
const GES_BACKWARD_FLAG: u8 = 1 << 5;
const GES_CLOCKWISE_FLAG: u8 = 1 << 6;
const GES_ANTI_CLOCKWISE_FLAG: u8 = 1 << 7;
const GES_WAVE_FLAG: u8 = 1 << 0;

/// grove.py's return_gesture() codes 1..9 mapped to the app's Gesture
/// enum names, in code order (code 0 = no gesture, hence no entry).
pub const GESTURE_NAMES: [&str; 9] = [
    "forward",
    "backward",
    "right",
    "left",
    "up",
    "down",
    "clockwise",
    "antiClockwise",
    "wave",
];

/// The gesture name for a return_gesture() code, or None for 0 (no
/// gesture) and codes the driver never reports.
pub fn gesture_name(code: u8) -> Option<&'static str> {
    if code == 0 {
        return None;
    }
    GESTURE_NAMES.get(usize::from(code) - 1).copied()
}

/// grove.py's initRegisterArray verbatim: the undocumented power-on
/// configuration from Seeed's reference driver, including the bank
/// switch ([0xEF, 0x01]) in the middle.
const INIT_REGISTERS: &[(u8, u8)] = &[
    (0xEF, 0x00), (0x32, 0x29), (0x33, 0x01), (0x34, 0x00), (0x35, 0x01),
    (0x36, 0x00), (0x37, 0x07), (0x38, 0x17), (0x39, 0x06), (0x3A, 0x12),
    (0x3F, 0x00), (0x40, 0x02), (0x41, 0xFF), (0x42, 0x01), (0x46, 0x2D),
    (0x47, 0x0F), (0x48, 0x3C), (0x49, 0x00), (0x4A, 0x1E), (0x4B, 0x00),
    (0x4C, 0x20), (0x4D, 0x00), (0x4E, 0x1A), (0x4F, 0x14), (0x50, 0x00),
    (0x51, 0x10), (0x52, 0x00), (0x5C, 0x02), (0x5D, 0x00), (0x5E, 0x10),
    (0x5F, 0x3F), (0x60, 0x27), (0x61, 0x28), (0x62, 0x00), (0x63, 0x03),
    (0x64, 0xF7), (0x65, 0x03), (0x66, 0xD9), (0x67, 0x03), (0x68, 0x01),
    (0x69, 0xC8), (0x6A, 0x40), (0x6D, 0x04), (0x6E, 0x00), (0x6F, 0x00),
    (0x70, 0x80), (0x71, 0x00), (0x72, 0x00), (0x73, 0x00), (0x74, 0xF0),
    (0x75, 0x00), (0x80, 0x42), (0x81, 0x44), (0x82, 0x04), (0x83, 0x20),
    (0x84, 0x20), (0x85, 0x00), (0x86, 0x10), (0x87, 0x00), (0x88, 0x05),
    (0x89, 0x18), (0x8A, 0x10), (0x8B, 0x01), (0x8C, 0x37), (0x8D, 0x00),
    (0x8E, 0xF0), (0x8F, 0x81), (0x90, 0x06), (0x91, 0x06), (0x92, 0x1E),
    (0x93, 0x0D), (0x94, 0x0A), (0x95, 0x0A), (0x96, 0x0C), (0x97, 0x05),
    (0x98, 0x0A), (0x99, 0x41), (0x9A, 0x14), (0x9B, 0x0A), (0x9C, 0x3F),
    (0x9D, 0x33), (0x9E, 0xAE), (0x9F, 0xF9), (0xA0, 0x48), (0xA1, 0x13),
    (0xA2, 0x10), (0xA3, 0x08), (0xA4, 0x30), (0xA5, 0x19), (0xA6, 0x10),
    (0xA7, 0x08), (0xA8, 0x24), (0xA9, 0x04), (0xAA, 0x1E), (0xAB, 0x1E),
    (0xCC, 0x19), (0xCD, 0x0B), (0xCE, 0x13), (0xCF, 0x64), (0xD0, 0x21),
    (0xD1, 0x0F), (0xD2, 0x88), (0xE0, 0x01), (0xE1, 0x04), (0xE2, 0x41),
    (0xE3, 0xD6), (0xE4, 0x00), (0xE5, 0x0C), (0xE6, 0x0A), (0xE7, 0x00),
    (0xE8, 0x00), (0xE9, 0x00), (0xEE, 0x07), (0xEF, 0x01), (0x00, 0x1E),
    (0x01, 0x1E), (0x02, 0x0F), (0x03, 0x10), (0x04, 0x02), (0x05, 0x00),
    (0x06, 0xB0), (0x07, 0x04), (0x08, 0x0D), (0x09, 0x0E), (0x0A, 0x9C),
    (0x0B, 0x04), (0x0C, 0x05), (0x0D, 0x0F), (0x0E, 0x02), (0x0F, 0x12),
    (0x10, 0x02), (0x11, 0x02), (0x12, 0x00), (0x13, 0x01), (0x14, 0x05),
    (0x15, 0x07), (0x16, 0x05), (0x17, 0x07), (0x18, 0x01), (0x19, 0x04),
    (0x1A, 0x05), (0x1B, 0x0C), (0x1C, 0x2A), (0x1D, 0x01), (0x1E, 0x00),
    (0x21, 0x00), (0x22, 0x00), (0x23, 0x00), (0x25, 0x01), (0x26, 0x00),
    (0x27, 0x39), (0x28, 0x7F), (0x29, 0x08), (0x30, 0x03), (0x31, 0x00),
    (0x32, 0x1A), (0x33, 0x1A), (0x34, 0x07), (0x35, 0x07), (0x36, 0x01),
    (0x37, 0xFF), (0x38, 0x36), (0x39, 0x07), (0x3A, 0x00), (0x3E, 0xFF),
    (0x3F, 0x00), (0x40, 0x77), (0x41, 0x40), (0x42, 0x00), (0x43, 0x30),
    (0x44, 0xA0), (0x45, 0x5C), (0x46, 0x00), (0x47, 0x00), (0x48, 0x58),
    (0x4A, 0x1E), (0x4B, 0x1E), (0x4C, 0x00), (0x4D, 0x00), (0x4E, 0xA0),
    (0x4F, 0x80), (0x50, 0x00), (0x51, 0x00), (0x52, 0x00), (0x53, 0x00),
    (0x54, 0x00), (0x57, 0x80), (0x59, 0x10), (0x5A, 0x08), (0x5B, 0x94),
    (0x5C, 0xE8), (0x5D, 0x08), (0x5E, 0x3D), (0x5F, 0x99), (0x60, 0x45),
    (0x61, 0x40), (0x63, 0x2D), (0x64, 0x02), (0x65, 0x96), (0x66, 0x00),
    (0x67, 0x97), (0x68, 0x01), (0x69, 0xCD), (0x6A, 0x01), (0x6B, 0xB0),
    (0x6C, 0x04), (0x6D, 0x2C), (0x6E, 0x01), (0x6F, 0x32), (0x71, 0x00),
    (0x72, 0x01), (0x73, 0x35), (0x74, 0x00), (0x75, 0x33), (0x76, 0x31),
    (0x77, 0x01), (0x7C, 0x84), (0x7D, 0x03), (0x7E, 0x01),
];

/// grove.py's return_gesture(): read the flag register; a directional
/// flag is held for GES_ENTRY_TIME and re-read, because a hand entering
/// the field fires right/left/up/down before the driver can tell a
/// forward/backward push. Returns the gesture code (0 = none). Reading
/// and sleeping are injected so the decoding is unit-testable.
pub fn decode_gesture(
    read: &mut dyn FnMut(u8) -> Result<u8, String>,
    sleep: &mut dyn FnMut(Duration),
) -> Result<u8, String> {
    let data = read(REG_FLAG_0)?;
    let directional = match data {
        GES_RIGHT_FLAG => Some(3),
        GES_LEFT_FLAG => Some(4),
        GES_UP_FLAG => Some(5),
        GES_DOWN_FLAG => Some(6),
        _ => None,
    };
    if let Some(code) = directional {
        sleep(GES_ENTRY_TIME);
        let data = read(REG_FLAG_0)?;
        return match data {
            GES_FORWARD_FLAG => {
                sleep(GES_QUIT_TIME);
                Ok(1)
            }
            GES_BACKWARD_FLAG => {
                sleep(GES_QUIT_TIME);
                Ok(2)
            }
            _ => Ok(code),
        };
    }
    match data {
        GES_FORWARD_FLAG => {
            sleep(GES_QUIT_TIME);
            return Ok(1);
        }
        GES_BACKWARD_FLAG => {
            sleep(GES_QUIT_TIME);
            return Ok(2);
        }
        GES_CLOCKWISE_FLAG => return Ok(7),
        GES_ANTI_CLOCKWISE_FLAG => return Ok(8),
        _ => {}
    }
    if read(REG_FLAG_1)? == GES_WAVE_FLAG {
        return Ok(9);
    }
    Ok(0)
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;
    use std::thread::sleep as thread_sleep;

    /// Reads gestures from a PAJ7620U2 on /dev/i2c-<bus> at 0x73.
    pub struct Paj7620 {
        dev: I2CDevice,
    }

    impl Paj7620 {
        /// Open the sensor and run the init sequence, waking the sensor
        /// if it does not answer: the PAJ7620 keeps its I2C interface
        /// powered down until bus activity wakes it, and does not
        /// acknowledge the transaction that does the waking — so the
        /// first register write of an unpoked sensor fails with EIO.
        /// Poke the bus, give the sensor a moment, and try again (like
        /// the Python node's _init_sensor).
        pub fn new(bus: u8) -> Result<Self, String> {
            let dev = I2CDevice::open(bus, ADDRESS)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let mut s = Self { dev };
            let mut last_error = String::new();
            for attempt in 0..WAKE_ATTEMPTS {
                match s.init() {
                    Ok(()) => return Ok(s),
                    Err(err) => last_error = err,
                }
                if attempt < WAKE_ATTEMPTS - 1 {
                    // One throwaway transaction wakes the sensor; it is
                    // itself not acknowledged, so its error is expected
                    // and ignored.
                    let _ = s.dev.read_reg(0x00);
                    thread_sleep(Duration::from_millis(50));
                }
            }
            Err(format!(
                "PAJ7620 did not respond on i2c bus {bus} ({last_error}). \
                 Check the wiring and that the sensor appears at 0x73"
            ))
        }

        /// grove.py's init(): select bank 0 (twice, the second select
        /// covers a sensor that ignored the waking first one), verify
        /// the part id and write the whole init register table.
        fn init(&mut self) -> Result<(), String> {
            thread_sleep(Duration::from_millis(1));
            self.write_reg(REG_BANK_SEL, 0)?;
            self.write_reg(REG_BANK_SEL, 0)?;
            let data0 = self.dev.read_reg(0).map_err(|e| e.to_string())?;
            self.dev.read_reg(1).map_err(|e| e.to_string())?;
            // grove.py only warns on an unexpected part id and carries on.
            if data0 != 0x20 {
                println!("Error with sensor");
            } else {
                println!("wake-up finish.");
            }
            for &(reg, value) in INIT_REGISTERS {
                self.write_reg(reg, value)?;
            }
            self.write_reg(REG_BANK_SEL, 0)?;
            println!("Paj7620 initialize register finished.");
            Ok(())
        }

        fn write_reg(&mut self, reg: u8, value: u8) -> Result<(), String> {
            self.dev.write_reg(reg, value).map_err(|e| e.to_string())
        }

        /// The detected gesture name, or None if nothing happened.
        pub fn read_gesture(&mut self) -> Result<Option<&'static str>, String> {
            let dev = &mut self.dev;
            let code = decode_gesture(
                &mut |reg| dev.read_reg(reg).map_err(|e| e.to_string()),
                &mut |d| thread_sleep(d),
            )?;
            Ok(gesture_name(code))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The gesture-code-to-name mapping, exactly the Python node's
    // GESTURES dict / the app's Gesture enum names.
    #[test]
    fn gesture_code_to_name_mapping() {
        let expected = [
            (1u8, "forward"),
            (2, "backward"),
            (3, "right"),
            (4, "left"),
            (5, "up"),
            (6, "down"),
            (7, "clockwise"),
            (8, "antiClockwise"),
            (9, "wave"),
        ];
        assert_eq!(GESTURE_NAMES.len(), expected.len());
        for (code, want) in expected {
            assert_eq!(gesture_name(code), Some(want), "code {code}");
        }
        // 0 means "no gesture" and must not map to a name; neither do
        // codes past the map.
        assert_eq!(gesture_name(0), None);
        assert_eq!(gesture_name(10), None);
    }

    /// Feeds decode_gesture a scripted sequence of (register, value)
    /// reads, asserting each register is the expected one.
    fn fake_reads(reads: &[(u8, u8)]) -> impl FnMut(u8) -> Result<u8, String> + '_ {
        let mut i = 0usize;
        move |reg| {
            assert!(i < reads.len(), "unexpected read of register {reg:#04x}");
            let (want_reg, value) = reads[i];
            assert_eq!(
                reg, want_reg,
                "read register {reg:#04x}, want {want_reg:#04x}"
            );
            i += 1;
            Ok(value)
        }
    }

    #[test]
    fn decodes_the_grove_py_flag_sequences() {
        let cases: [(&str, &[(u8, u8)], u8); 12] = [
            ("right", &[(0x43, 0x01), (0x43, 0x00)], 3),
            ("left", &[(0x43, 0x02), (0x43, 0x00)], 4),
            ("up", &[(0x43, 0x04), (0x43, 0x00)], 5),
            ("down", &[(0x43, 0x08), (0x43, 0x00)], 6),
            ("forward", &[(0x43, 0x10)], 1),
            ("backward", &[(0x43, 0x20)], 2),
            ("forward after right", &[(0x43, 0x01), (0x43, 0x10)], 1),
            ("backward after up", &[(0x43, 0x04), (0x43, 0x20)], 2),
            ("clockwise", &[(0x43, 0x40)], 7),
            ("anticlockwise", &[(0x43, 0x80)], 8),
            ("wave", &[(0x43, 0x00), (0x44, 0x01)], 9),
            ("nothing", &[(0x43, 0x00), (0x44, 0x00)], 0),
        ];
        for (name, reads, want) in cases {
            let mut read = fake_reads(reads);
            let got = decode_gesture(&mut read, &mut |_| {}).unwrap();
            assert_eq!(got, want, "{name}: code {got}, want {want}");
        }
    }

    // A combined gesture waits GES_ENTRY_TIME before the re-read and
    // GES_QUIT_TIME afterwards, exactly like grove.py.
    #[test]
    fn combined_gesture_waits_entry_then_quit_time() {
        let mut slept = Vec::new();
        let mut read = fake_reads(&[(0x43, 0x01), (0x43, 0x10)]);
        let code = decode_gesture(&mut read, &mut |d| slept.push(d)).unwrap();
        assert_eq!(code, 1);
        assert_eq!(slept, [GES_ENTRY_TIME, GES_QUIT_TIME]);
    }

    // A read error surfaces instead of a bogus gesture.
    #[test]
    fn propagates_read_errors() {
        let err = decode_gesture(&mut |_| Err("EIO".into()), &mut |_| {}).unwrap_err();
        assert_eq!(err, "EIO");
    }
}
