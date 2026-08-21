//! AT24C128 text-region access — the record format plus the I2C side of a
//! 128 Kbit (16 KB) serial EEPROM with 16-bit memory addressing.
//!
//! The record lives at offset 0: magic `'S' 'P'`, a u16 big-endian byte
//! length (max 512), then the UTF-8 text. A chip without the magic (e.g.
//! factory-fresh, all 0xFF) reads as an empty text rather than as garbage.
//!
//! The common crate's I2CDevice speaks 8-bit register maps, so the chip
//! gets its own minimal wrapper (Linux only): an I2C_SLAVE ioctl, then a
//! two-byte memory address in front of every transaction. The Python node
//! reads a chunk in one combined i2c_rdwr transaction; the AT24C128's
//! address counter survives the stop condition in between, so separate
//! write/read transactions return the same bytes.

/// Magic marking a text region written by Sensor Playground.
const MAGIC: [u8; 2] = [b'S', b'P'];

/// 2-byte magic + 2-byte big-endian length.
pub const HEADER_SIZE: usize = 4;

/// Longest text the chip region holds, in UTF-8 bytes.
pub const TEXT_MAX_BYTES: usize = 512;

/// What the controller stores the text on: the real chip or the emulation.
pub trait Eeprom: Send {
    fn read_text(&mut self) -> Result<String, String>;
    fn write_text(&mut self, text: &[u8]) -> Result<(), String>;
}

// -- Record encoding ---------------------------------------------------------

/// The bytes stored at offset 0 for `text`: the magic, the big-endian byte
/// length and the UTF-8 text itself.
pub fn encode_record(text: &[u8]) -> Vec<u8> {
    let mut record = Vec::with_capacity(HEADER_SIZE + text.len());
    record.extend_from_slice(&MAGIC);
    record.extend_from_slice(&(text.len() as u16).to_be_bytes());
    record.extend_from_slice(text);
    record
}

/// The stored text length read from a header, or `None` when the chip
/// holds no usable record: a missing magic or an implausible length reads
/// as an empty text rather than as garbage.
pub fn record_length(header: &[u8]) -> Option<usize> {
    if header.len() < HEADER_SIZE || header[..2] != MAGIC {
        return None;
    }
    let length = usize::from(u16::from_be_bytes([header[2], header[3]]));
    (length <= TEXT_MAX_BYTES).then_some(length)
}

/// Parses a whole record (header + payload) into the stored text — the
/// pure counterpart of [`At24c128Eeprom::read_text`].
pub fn decode_record(record: &[u8]) -> String {
    let Some(length) = record_length(record) else {
        return String::new();
    };
    // A truncated dump reports what is there; on the chip the read always
    // returns the full declared length.
    let end = (HEADER_SIZE + length).min(record.len());
    decode_utf8_replace(&record[HEADER_SIZE..end])
}

/// Decodes UTF-8 with U+FFFD for each invalid byte, like Python's
/// errors="replace".
pub fn decode_utf8_replace(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        match std::str::from_utf8(rest) {
            Ok(s) => {
                out.push_str(s);
                break;
            }
            Err(err) => {
                let valid = err.valid_up_to();
                out.push_str(std::str::from_utf8(&rest[..valid]).unwrap());
                out.push(char::REPLACEMENT_CHARACTER);
                rest = &rest[valid + 1..];
            }
        }
    }
    out
}

// -- Hardware ----------------------------------------------------------------

#[cfg(target_os = "linux")]
mod hardware {
    use super::{decode_record, encode_record, record_length, Eeprom, HEADER_SIZE};
    use std::fs::{File, OpenOptions};
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::Duration;

    const I2C_SLAVE: libc::c_ulong = 0x0703;

    /// AT24C128 write page; a write transaction must not cross a page
    /// boundary or it wraps around inside the page.
    const PAGE_SIZE: usize = 64;

    /// Bytes per I2C transaction, safe on every adapter.
    const IO_CHUNK: usize = 32;

    /// Worst-case internal write cycle per the datasheet is 5 ms.
    const WRITE_CYCLE: Duration = Duration::from_millis(6);

    /// Reads and writes the AT24C128 text region over I2C.
    pub struct At24c128Eeprom {
        f: File,
    }

    impl At24c128Eeprom {
        /// Opens the chip and reads it once, failing fast on a
        /// wiring/address problem (like the Python node's constructor).
        pub fn open(bus: u8, addr: u16) -> std::io::Result<Self> {
            let f = OpenOptions::new()
                .read(true)
                .write(true)
                .open(format!("/dev/i2c-{bus}"))?;
            // SAFETY: a plain ioctl on an owned fd.
            let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, addr as libc::c_ulong) };
            if rc != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut eeprom = Self { f };
            eeprom.read_bytes(0, HEADER_SIZE)?;
            Ok(eeprom)
        }

        /// Random-reads `length` bytes starting at `addr`.
        fn read_bytes(&mut self, addr: usize, length: usize) -> std::io::Result<Vec<u8>> {
            let mut data = vec![0u8; length];
            for offset in (0..length).step_by(IO_CHUNK) {
                let at = addr + offset;
                let count = IO_CHUNK.min(length - offset);
                self.f.write_all(&[(at >> 8) as u8, (at & 0xFF) as u8])?;
                self.f.read_exact(&mut data[offset..offset + count])?;
            }
            Ok(data)
        }

        /// Writes `data` starting at `addr`, splitting the transactions so
        /// none crosses a 64-byte page boundary, and waiting out the
        /// chip's internal write cycle after each one.
        fn write_bytes(&mut self, addr: usize, data: &[u8]) -> std::io::Result<()> {
            let mut offset = 0;
            while offset < data.len() {
                let at = addr + offset;
                let count = IO_CHUNK
                    .min(PAGE_SIZE - at % PAGE_SIZE)
                    .min(data.len() - offset);
                let mut frame = Vec::with_capacity(2 + count);
                frame.extend_from_slice(&[(at >> 8) as u8, (at & 0xFF) as u8]);
                frame.extend_from_slice(&data[offset..offset + count]);
                self.f.write_all(&frame)?;
                std::thread::sleep(WRITE_CYCLE);
                offset += count;
            }
            Ok(())
        }
    }

    impl Eeprom for At24c128Eeprom {
        /// Reads the stored text from the chip. A missing magic or an
        /// implausible length reads as an empty text.
        fn read_text(&mut self) -> Result<String, String> {
            let mut record = self
                .read_bytes(0, HEADER_SIZE)
                .map_err(|err| err.to_string())?;
            let Some(length) = record_length(&record) else {
                return Ok(String::new());
            };
            let text = self
                .read_bytes(HEADER_SIZE, length)
                .map_err(|err| err.to_string())?;
            record.extend_from_slice(&text);
            Ok(decode_record(&record))
        }

        /// Stores the UTF-8 `text` (header + payload) on the chip.
        fn write_text(&mut self, text: &[u8]) -> Result<(), String> {
            self.write_bytes(0, &encode_record(text))
                .map_err(|err| err.to_string())
        }
    }
}

#[cfg(target_os = "linux")]
pub use hardware::At24c128Eeprom;

// -- Emulation ---------------------------------------------------------------

/// Keeps the text in memory instead of driving hardware.
pub struct EmulatedEeprom {
    text: String,
}

impl EmulatedEeprom {
    pub fn new() -> Self {
        Self {
            text: "Hello from the emulated EEPROM".into(),
        }
    }
}

impl Eeprom for EmulatedEeprom {
    /// Returns the fake stored text.
    fn read_text(&mut self) -> Result<String, String> {
        Ok(self.text.clone())
    }

    /// Stores the UTF-8 `text` in memory.
    fn write_text(&mut self, text: &[u8]) -> Result<(), String> {
        self.text = decode_utf8_replace(text);
        println!("[emulation] stored {} bytes", text.len());
        Ok(())
    }
}

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The same vectors as the Go and Node.js ports.
    #[test]
    fn encode_record_matches_the_layout() {
        assert_eq!(encode_record(b"Hi"), b"SP\x00\x02Hi".to_vec());
        assert_eq!(encode_record(b""), b"SP\x00\x00".to_vec());
        // "äöü" is 6 UTF-8 bytes, not 3 characters.
        assert_eq!(
            encode_record("äöü".as_bytes()),
            b"SP\x00\x06\xC3\xA4\xC3\xB6\xC3\xBC".to_vec()
        );
    }

    /// A length above 255 must land in the high byte of the header.
    #[test]
    fn encode_record_long_length() {
        let record = encode_record(&vec![b'a'; 300]);
        assert_eq!(&record[..4], b"SP\x01\x2C");
        assert_eq!(record.len(), HEADER_SIZE + 300);
    }

    #[test]
    fn record_length_reads_the_header() {
        assert_eq!(record_length(b"SP\x00\x05"), Some(5));
        assert_eq!(record_length(b"SP\x00\x00"), Some(0));
        assert_eq!(record_length(b"SP\x02\x00"), Some(TEXT_MAX_BYTES));
        // A factory-fresh chip is all 0xFF and holds no record.
        assert_eq!(record_length(&[0xFF; 4]), None);
        assert_eq!(record_length(&[0x00; 4]), None);
        assert_eq!(record_length(b"XP\x00\x02"), None);
        // An implausible length reads as no record rather than as garbage.
        assert_eq!(record_length(b"SP\x02\x01"), None);
        assert_eq!(record_length(b"SP\x00"), None);
    }

    #[test]
    fn decode_record_matches_the_python_node() {
        assert_eq!(decode_record(b"SP\x00\x02Hi"), "Hi");
        assert_eq!(decode_record(b"SP\x00\x00"), "");
        assert_eq!(decode_record(&[0xFF; 16]), "");
        assert_eq!(decode_record(b"NO\x00\x02Hi"), "");
        assert_eq!(decode_record(b"SP\xFF\xFFHi"), "");
        // Only the declared length is text; trailing bytes are ignored.
        assert_eq!(decode_record(b"SP\x00\x02HiXY"), "Hi");
        assert_eq!(decode_record(b"SP\x00\x05\xE2\x98\x95\xC3\xA4"), "☕ä");
        // Invalid bytes become U+FFFD, like Python's errors="replace".
        assert_eq!(decode_record(b"SP\x00\x03a\xFFb"), "a\u{FFFD}b");
    }

    #[test]
    fn encode_decode_round_trip() {
        for text in ["", "Hello EEPROM", "äöü ☕", &"x".repeat(TEXT_MAX_BYTES)] {
            assert_eq!(decode_record(&encode_record(text.as_bytes())), text);
        }
    }

    #[test]
    fn emulation_stores_and_returns_the_text() {
        let mut eeprom = EmulatedEeprom::new();
        assert_eq!(
            eeprom.read_text().unwrap(),
            "Hello from the emulated EEPROM"
        );
        eeprom.write_text("äöü".as_bytes()).unwrap();
        assert_eq!(eeprom.read_text().unwrap(), "äöü");
    }
}
