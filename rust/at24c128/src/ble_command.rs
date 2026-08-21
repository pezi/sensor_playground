//! BLE command framing for the EEPROM node — a port of the Python node's
//! BleCommandStream.
//!
//! A text may not fit in a single ATT write, so a write is staged in
//! offset-addressed binary chunks on the command characteristic (like the
//! SSD1306 bitmap):
//!
//!     0x01 <offset:u16 big-endian> <bytes...>   stage a chunk of UTF-8 text
//!     0x02 <length:u16 big-endian>              store the staged text
//!     0x03                                      re-read the chip and push
//!
//! Chunks carry an absolute offset and must arrive contiguously. The
//! offset is what makes the protocol self-synchronising: staging offset 0
//! starts a new transfer, and the store command carries the total length,
//! so a dropped packet refuses the write instead of storing a torn text.

use crate::eeprom::TEXT_MAX_BYTES;

const CMD_CHUNK: u8 = 0x01; // 0x01 <offset:u16 big-endian> <bytes...>
const CMD_WRITE: u8 = 0x02; // 0x02 <length:u16 big-endian>
const CMD_READ: u8 = 0x03; // re-read the chip and push

/// What a completed command packet asks the controller to do.
#[derive(Debug, PartialEq, Eq)]
pub enum BleCommand {
    /// Store this UTF-8 text on the chip.
    Write(Vec<u8>),
    /// Re-read the chip and push.
    Read,
}

/// Assembles the command-characteristic writes into commands.
pub struct BleCommandStream {
    buffer: Vec<u8>,
    staged: usize,
}

impl BleCommandStream {
    pub fn new() -> Self {
        Self {
            buffer: vec![0u8; TEXT_MAX_BYTES],
            staged: 0,
        }
    }

    /// Handles one command packet: `Ok(None)` while a chunk is only
    /// staged, `Ok(Some(..))` once a packet completes a command, and
    /// `Err` on a bad one.
    pub fn feed(&mut self, packet: &[u8]) -> Result<Option<BleCommand>, String> {
        let Some(&opcode) = packet.first() else {
            return Err("empty command packet".into());
        };
        match opcode {
            CMD_READ => Ok(Some(BleCommand::Read)),
            CMD_WRITE => self.store(packet).map(Some),
            CMD_CHUNK => {
                self.stage(packet)?;
                Ok(None)
            }
            _ => Err(format!("unknown opcode {opcode:#04x}")),
        }
    }

    fn store(&mut self, packet: &[u8]) -> Result<BleCommand, String> {
        if packet.len() < 3 {
            return Err("write without a length".into());
        }
        let total = usize::from(u16::from_be_bytes([packet[1], packet[2]]));
        let staged = std::mem::take(&mut self.staged);
        if total != staged {
            return Err(format!("write of {total} bytes with {staged} staged"));
        }
        Ok(BleCommand::Write(self.buffer[..total].to_vec()))
    }

    fn stage(&mut self, packet: &[u8]) -> Result<(), String> {
        if packet.len() < 3 {
            return Err("chunk without an offset".into());
        }
        let offset = usize::from(u16::from_be_bytes([packet[1], packet[2]]));
        let data = &packet[3..];
        if offset + data.len() > TEXT_MAX_BYTES {
            return Err(format!(
                "chunk at offset {offset} overruns the text region ({} bytes)",
                data.len()
            ));
        }
        if offset != self.staged {
            if offset != 0 {
                let expected = std::mem::take(&mut self.staged);
                return Err(format!(
                    "chunk at offset {offset} is not contiguous (expected {expected})"
                ));
            }
            // Offset 0 always starts a new transfer.
            self.staged = 0;
        }
        self.buffer[offset..offset + data.len()].copy_from_slice(data);
        self.staged = offset + data.len();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `0x01 <offset> <bytes>`
    fn chunk(offset: u16, data: &[u8]) -> Vec<u8> {
        let mut packet = vec![CMD_CHUNK];
        packet.extend_from_slice(&offset.to_be_bytes());
        packet.extend_from_slice(data);
        packet
    }

    /// `0x02 <length>`
    fn store(length: u16) -> Vec<u8> {
        let mut packet = vec![CMD_WRITE];
        packet.extend_from_slice(&length.to_be_bytes());
        packet
    }

    #[test]
    fn single_chunk_write() {
        let mut stream = BleCommandStream::new();
        assert_eq!(stream.feed(&chunk(0, b"Hello")), Ok(None));
        assert_eq!(
            stream.feed(&store(5)),
            Ok(Some(BleCommand::Write(b"Hello".to_vec())))
        );
    }

    #[test]
    fn contiguous_chunks_are_assembled() {
        let mut stream = BleCommandStream::new();
        stream.feed(&chunk(0, b"Hello ")).unwrap();
        stream.feed(&chunk(6, b"EEPROM")).unwrap();
        assert_eq!(
            stream.feed(&store(12)),
            Ok(Some(BleCommand::Write(b"Hello EEPROM".to_vec())))
        );
    }

    #[test]
    fn empty_text_writes() {
        let mut stream = BleCommandStream::new();
        assert_eq!(
            stream.feed(&store(0)),
            Ok(Some(BleCommand::Write(Vec::new())))
        );
    }

    #[test]
    fn read_command() {
        let mut stream = BleCommandStream::new();
        assert_eq!(stream.feed(&[CMD_READ]), Ok(Some(BleCommand::Read)));
        // Trailing bytes are ignored, like the Python node's opcode check.
        assert_eq!(stream.feed(&[CMD_READ, 0xAA]), Ok(Some(BleCommand::Read)));
    }

    #[test]
    fn multi_byte_utf8_survives_a_chunk_boundary() {
        // "äöü" split in the middle of the second character's bytes.
        let text = "äöü".as_bytes();
        let mut stream = BleCommandStream::new();
        stream.feed(&chunk(0, &text[..3])).unwrap();
        stream.feed(&chunk(3, &text[3..])).unwrap();
        assert_eq!(
            stream.feed(&store(6)),
            Ok(Some(BleCommand::Write(text.to_vec())))
        );
    }

    #[test]
    fn a_gap_refuses_the_write() {
        let mut stream = BleCommandStream::new();
        stream.feed(&chunk(0, b"Hello")).unwrap();
        // The chunk at offset 5 was dropped on the air.
        assert_eq!(
            stream.feed(&chunk(9, b"world")),
            Err("chunk at offset 9 is not contiguous (expected 5)".into())
        );
        // The refused chunk cleared the staging, so the store fails too.
        assert_eq!(
            stream.feed(&store(5)),
            Err("write of 5 bytes with 0 staged".into())
        );
    }

    #[test]
    fn offset_zero_restarts_the_transfer() {
        let mut stream = BleCommandStream::new();
        stream.feed(&chunk(0, b"stale text")).unwrap();
        stream.feed(&chunk(0, b"new")).unwrap();
        assert_eq!(
            stream.feed(&store(3)),
            Ok(Some(BleCommand::Write(b"new".to_vec())))
        );
    }

    #[test]
    fn a_length_mismatch_refuses_the_write() {
        let mut stream = BleCommandStream::new();
        stream.feed(&chunk(0, b"Hello")).unwrap();
        assert_eq!(
            stream.feed(&store(4)),
            Err("write of 4 bytes with 5 staged".into())
        );
    }

    #[test]
    fn a_chunk_may_not_overrun_the_text_region() {
        let mut stream = BleCommandStream::new();
        assert_eq!(
            stream.feed(&chunk(TEXT_MAX_BYTES as u16 - 2, b"abcd")),
            Err("chunk at offset 510 overruns the text region (4 bytes)".into())
        );
        // The whole region can be filled exactly.
        assert!(stream
            .feed(&chunk(0, &vec![b'x'; TEXT_MAX_BYTES]))
            .is_ok());
        assert_eq!(
            stream.feed(&store(TEXT_MAX_BYTES as u16)),
            Ok(Some(BleCommand::Write(vec![b'x'; TEXT_MAX_BYTES])))
        );
    }

    #[test]
    fn malformed_packets() {
        let mut stream = BleCommandStream::new();
        assert_eq!(stream.feed(&[]), Err("empty command packet".into()));
        assert_eq!(
            stream.feed(&[CMD_CHUNK, 0x00]),
            Err("chunk without an offset".into())
        );
        assert_eq!(
            stream.feed(&[CMD_WRITE, 0x00]),
            Err("write without a length".into())
        );
        assert_eq!(stream.feed(&[0x99]), Err("unknown opcode 0x99".into()));
    }
}
