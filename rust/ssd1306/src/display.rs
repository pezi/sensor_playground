//! SSD1306 128x64 OLED driver — a port of the Python node's, which is
//! itself a port of dart_periphery's SSD1306: the same init sequence, the
//! same horizontal-addressing window, and the same bitmap transposition.
//!
//! The bitmap transposition is platform-neutral (and unit tested); only the
//! I2C access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 64;
/// One bit per pixel: 1024 bytes for the whole panel.
pub const FRAME_SIZE: usize = WIDTH * HEIGHT / 8;
/// 16 bytes per row in the app's (horizontal) bitmap format.
pub const ROW_BYTES: usize = WIDTH / 8;

const CONTROL_COMMAND: u8 = 0x00; // the byte that follows is a command
const CONTROL_DATA: u8 = 0x40; // the bytes that follow are display RAM

/// The same init sequence the dart_periphery SSD1306 driver sends.
const INIT_SEQUENCE: [u8; 25] = [
    0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00, 0xA1, 0xC8, 0xDA, 0x12,
    0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6, 0xAF,
];

/// Transpose a horizontal MSB-first bitmap into SSD1306 page format.
///
/// The app sends what https://javl.github.io/image2cpp/ calls "horizontal"
/// byte orientation: 16 bytes per row, the MSB of each byte is the leftmost
/// pixel. The panel wants 8 pages of 128 column bytes, each byte holding 8
/// *vertical* pixels with the topmost in the LSB — so every output byte is
/// assembled from one bit of eight different input rows.
pub fn to_native_format(data: &[u8]) -> Vec<u8> {
    let mut buffer = vec![0u8; FRAME_SIZE];
    let mut count = 0;
    let mut index = 0;
    for _page in 0..HEIGHT / 8 {
        for column_byte in 0..ROW_BYTES {
            let pos = index + column_byte;
            for bit in (0..8).rev() {
                let mask = 1u8 << bit;
                let mut value = 0u8;
                for row in 0..8 {
                    if data[pos + ROW_BYTES * row] & mask != 0 {
                        value |= 1 << row;
                    }
                }
                buffer[count] = value;
                count += 1;
            }
        }
        index += WIDTH;
    }
    buffer
}

/// What the command handlers drive: the real panel or the emulated one.
pub trait Display: Send {
    fn clear(&mut self) -> Result<(), String>;
    fn show_bitmap(&mut self, data: &[u8]) -> Result<(), String>;
}

/// Reports what would be drawn instead of driving hardware. A 1024-byte
/// frame is meaningless on a console, so it reports the share of lit pixels
/// — enough to tell a blank frame from a drawn one.
pub struct EmulatedDisplay;

impl Display for EmulatedDisplay {
    fn clear(&mut self) -> Result<(), String> {
        println!("[emulation] display cleared");
        Ok(())
    }

    fn show_bitmap(&mut self, data: &[u8]) -> Result<(), String> {
        let lit: u32 = to_native_format(data)
            .iter()
            .map(|byte| byte.count_ones())
            .sum();
        println!(
            "[emulation] display bitmap: {lit} of {} pixels lit",
            WIDTH * HEIGHT
        );
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub mod hw {
    use super::*;
    use common::i2c::I2CDevice;

    /// Drives the panel over I2C.
    pub struct Ssd1306 {
        dev: I2CDevice,
    }

    impl Ssd1306 {
        /// Open the panel on /dev/i2c-<bus>, initialize it and blank it.
        pub fn new(bus: u8, address: u16) -> Result<Self, String> {
            let dev = I2CDevice::open(bus, address)
                .map_err(|e| format!("opening /dev/i2c-{bus}: {e}"))?;
            let mut display = Self { dev };
            display.write(CONTROL_COMMAND, &INIT_SEQUENCE)?;
            display.clear()?;
            Ok(display)
        }

        /// Send a control byte followed by its payload in one transfer.
        fn write(&mut self, control: u8, data: &[u8]) -> Result<(), String> {
            let mut message = Vec::with_capacity(data.len() + 1);
            message.push(control);
            message.extend_from_slice(data);
            self.dev.write_bytes(&message).map_err(|e| e.to_string())
        }

        /// Point the panel's write cursor back at the top left by setting
        /// the full column (0-127) and page (0-7) range for horizontal
        /// addressing mode.
        fn reset_position(&mut self) -> Result<(), String> {
            self.write(CONTROL_COMMAND, &[0x21, 0x00, 0x7F, 0x22, 0x00, 0x07])
        }
    }

    impl Display for Ssd1306 {
        fn clear(&mut self) -> Result<(), String> {
            self.reset_position()?;
            self.write(CONTROL_DATA, &vec![0u8; FRAME_SIZE])
        }

        fn show_bitmap(&mut self, data: &[u8]) -> Result<(), String> {
            self.reset_position()?;
            self.write(CONTROL_DATA, &to_native_format(data))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The top-left pixel is the MSB of the first byte in the app's format,
    /// and the LSB of the first column byte in the panel's.
    #[test]
    fn single_pixel_top_left() {
        let mut data = vec![0u8; FRAME_SIZE];
        data[0] = 0x80;
        let native = to_native_format(&data);
        assert_eq!(native[0], 0x01);
        assert!(
            native[1..].iter().all(|byte| *byte == 0),
            "only one pixel is lit"
        );
    }

    /// Row 7, x = 127: the last row of page 0 (the MSB of a column byte) at
    /// the last column, which is the last byte of the first page.
    #[test]
    fn last_pixel_of_the_first_page() {
        let mut data = vec![0u8; FRAME_SIZE];
        data[7 * ROW_BYTES + 15] = 0x01;
        assert_eq!(to_native_format(&data)[127], 0x80);
    }

    /// All white stays all white; alternating rows become 0b01010101, since
    /// each column byte holds eight vertical pixels.
    #[test]
    fn uniform_patterns() {
        let white = vec![0xFFu8; FRAME_SIZE];
        assert!(to_native_format(&white).iter().all(|byte| *byte == 0xFF));

        let mut stripes = vec![0u8; FRAME_SIZE];
        for row in (0..HEIGHT).step_by(2) {
            for i in 0..ROW_BYTES {
                stripes[row * ROW_BYTES + i] = 0xFF;
            }
        }
        assert!(to_native_format(&stripes).iter().all(|byte| *byte == 0x55));
    }

    /// A full pseudo-random frame, checked against the reference
    /// implementation by a checksum — the uniform patterns above cannot
    /// catch a transposition that scrambles bytes within a page.
    #[test]
    fn matches_the_reference_frame() {
        let data: Vec<u8> = (0..FRAME_SIZE)
            .map(|i| ((i * 37 + 11) % 256) as u8)
            .collect();
        let native = to_native_format(&data);
        // Ends and byte sum taken from the Python node's to_native_format()
        // on the same input.
        assert_eq!(&native[..8], &[108, 90, 204, 170, 255, 0, 255, 255]);
        assert_eq!(
            &native[FRAME_SIZE - 8..],
            &[217, 180, 153, 85, 0, 255, 255, 0]
        );
        let sum: u64 = native.iter().map(|byte| u64::from(*byte)).sum();
        assert_eq!(sum, 130936);
        // A transposition moves pixels, it never adds or drops any.
        let lit_before: u32 = data.iter().map(|byte| byte.count_ones()).sum();
        let lit_after: u32 = native.iter().map(|byte| byte.count_ones()).sum();
        assert_eq!(lit_before, lit_after);
    }
}
