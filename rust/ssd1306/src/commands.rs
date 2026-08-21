//! Command parsing for the display node: JSON over the WebSocket, and the
//! chunked binary form BLE needs. Both are ports of the Python node's.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use crate::display::{Display, FRAME_SIZE};
use common::Payload;
use serde_json::json;

// -- WebSocket commands ---------------------------------------------------

fn ack(id: Option<i64>, ok: bool, error: Option<&str>) -> Payload {
    let mut p = Payload::new();
    p.insert("id".into(), id.map_or(json!(null), |id| json!(id)));
    p.insert("ok".into(), json!(ok));
    if let Some(error) = error {
        p.insert("error".into(), json!(error));
    }
    p
}

/// Execute one JSON command and return its ACK/NACK. The app matches the
/// reply to its command by id, so an id is required even for a command
/// that fails validation.
pub fn handle_json_command(display: &mut dyn Display, message: &str) -> Payload {
    let Ok(command) = serde_json::from_str::<serde_json::Value>(message) else {
        println!("Ignoring malformed command");
        return ack(None, false, Some("malformed JSON"));
    };
    if !command.is_object() {
        return ack(None, false, Some("command must be an object"));
    }
    let Some(id) = command.get("id").and_then(|v| v.as_i64()) else {
        return ack(None, false, Some("missing command id"));
    };

    if command.get("clear") == Some(&json!(true)) {
        println!("Command: clear");
        return match display.clear() {
            Ok(()) => ack(Some(id), true, None),
            Err(err) => ack(Some(id), false, Some(&err)),
        };
    }

    let Some(encoded) = command.get("image").and_then(|v| v.as_str()) else {
        return ack(Some(id), false, Some("unknown command"));
    };
    let Some(data) = decode_base64(encoded) else {
        println!("Ignoring command with invalid base64 image");
        return ack(Some(id), false, Some("invalid base64"));
    };
    if data.len() != FRAME_SIZE {
        println!(
            "Ignoring image with {} bytes (need {FRAME_SIZE})",
            data.len()
        );
        return ack(
            Some(id),
            false,
            Some(&format!(
                "image has {} bytes; need {FRAME_SIZE}",
                data.len()
            )),
        );
    }
    println!("Command: image");
    match display.show_bitmap(&data) {
        Ok(()) => ack(Some(id), true, None),
        Err(err) => ack(Some(id), false, Some(&err)),
    }
}

/// Decode standard base64 with padding, rejecting anything outside the
/// alphabet — the same strictness as the Python node's
/// b64decode(validate=True). Hand-rolled to keep the node dependency-free.
fn decode_base64(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return None;
    }
    let value_of = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (index, quad) in bytes.chunks(4).enumerate() {
        let last = index == bytes.len() / 4 - 1;
        // Padding is only allowed in the final quad, at most two bytes.
        let padding = if last {
            quad.iter().filter(|c| **c == b'=').count()
        } else {
            0
        };
        if padding > 2 || quad[..4 - padding].iter().any(|c| *c == b'=') {
            return None;
        }
        let mut accumulator = 0u32;
        for c in &quad[..4 - padding] {
            accumulator = accumulator << 6 | value_of(*c)?;
        }
        accumulator <<= 6 * padding;
        let decoded = accumulator.to_be_bytes();
        out.extend_from_slice(&decoded[1..4 - padding]);
    }
    Some(out)
}

// -- BLE command framing --------------------------------------------------

pub const CMD_CHUNK: u8 = 0x01; // 0x01 <offset:u16 big-endian> <bytes...>
pub const CMD_SHOW: u8 = 0x02; // draw the staged frame
pub const CMD_CLEAR: u8 = 0x03; // blank the display

/// What a fed command packet asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameAction {
    /// The packet only staged data.
    None,
    Show,
    Clear,
}

/// Reassembles a bitmap from the app's chunked BLE writes.
///
/// BLE has no equivalent of a WebSocket text frame: one ATT write carries
/// at most MTU-3 bytes, so a 1024-byte frame arrives as several writes that
/// the node has to stage. The chunks are also sent raw rather than
/// base64-encoded (BLE writes are binary-safe), which keeps the transfer at
/// 1024 bytes instead of the ~1.4 KB the JSON/base64 form would cost.
///
/// Chunks carry an absolute offset and must arrive contiguously. The offset
/// is what makes the protocol self-synchronising: writing offset 0 starts a
/// new frame, so a client that reconnects or gives up mid-frame simply
/// restarts and never has to be told to reset. The contiguity check is what
/// makes a partial frame detectable — without it a dropped chunk would be
/// drawn as a band of stale pixels from the previous image.
pub struct FrameAssembler {
    frame_size: usize,
    buffer: Vec<u8>,
    staged: usize,
}

impl FrameAssembler {
    pub fn new(frame_size: usize) -> Self {
        Self {
            frame_size,
            buffer: vec![0u8; frame_size],
            staged: 0,
        }
    }

    /// Consume one command packet. A returned FrameAction::Show means
    /// frame() holds a complete bitmap.
    pub fn feed(&mut self, packet: &[u8]) -> Result<FrameAction, String> {
        let Some(&opcode) = packet.first() else {
            return Err("empty command packet".into());
        };
        match opcode {
            CMD_CLEAR => {
                self.staged = 0;
                return Ok(FrameAction::Clear);
            }
            CMD_SHOW => {
                if self.staged != self.frame_size {
                    let staged = self.staged;
                    self.staged = 0;
                    return Err(format!(
                        "show with an incomplete frame ({staged} of {} bytes)",
                        self.frame_size
                    ));
                }
                self.staged = 0;
                return Ok(FrameAction::Show);
            }
            CMD_CHUNK => {}
            other => return Err(format!("unknown opcode {other:#04x}")),
        }

        if packet.len() < 3 {
            return Err("chunk without an offset".into());
        }
        let offset = usize::from(packet[1]) << 8 | usize::from(packet[2]);
        let data = &packet[3..];
        if offset + data.len() > self.frame_size {
            return Err(format!(
                "chunk at offset {offset} overruns the frame ({} bytes)",
                data.len()
            ));
        }
        if offset != self.staged && offset != 0 {
            // Offset 0 is a deliberate restart, anything else is a gap.
            let expected = self.staged;
            self.staged = 0;
            return Err(format!(
                "chunk at offset {offset} is not contiguous (expected {expected})"
            ));
        }
        self.buffer[offset..offset + data.len()].copy_from_slice(data);
        self.staged = offset + data.len();
        Ok(FrameAction::None)
    }

    /// The staged bitmap, valid right after feed() returned Show.
    pub fn frame(&self) -> &[u8] {
        &self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::display::EmulatedDisplay;

    fn chunk(offset: usize, data: &[u8]) -> Vec<u8> {
        let mut packet = vec![CMD_CHUNK, (offset >> 8) as u8, offset as u8];
        packet.extend_from_slice(data);
        packet
    }

    #[test]
    fn stages_contiguous_chunks() {
        let mut assembler = FrameAssembler::new(8);
        for packet in [chunk(0, &[1, 2, 3, 4]), chunk(4, &[5, 6, 7, 8])] {
            assert_eq!(assembler.feed(&packet), Ok(FrameAction::None));
        }
        assert_eq!(assembler.feed(&[CMD_SHOW]), Ok(FrameAction::Show));
        assert_eq!(assembler.frame(), &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn rejects_bad_packets() {
        let mut assembler = FrameAssembler::new(8);
        // A gap would otherwise be drawn as a band of stale pixels.
        assert!(assembler.feed(&chunk(4, &[1, 2])).is_err());
        // Showing a partial frame is an error, and it clears the staging.
        assert!(assembler.feed(&chunk(0, &[1, 2])).is_ok());
        assert!(assembler.feed(&[CMD_SHOW]).is_err());
        assert!(assembler.feed(&[]).is_err(), "empty packet");
        assert!(assembler.feed(&[0x7F]).is_err(), "unknown opcode");
        assert!(assembler.feed(&[CMD_CHUNK, 0x00]).is_err(), "short offset");
        assert!(
            assembler.feed(&chunk(6, &[1, 2, 3, 4])).is_err(),
            "overruns the frame"
        );
    }

    /// Writing offset 0 restarts the frame — that is how a client that
    /// reconnected mid-frame resynchronises without being told to reset.
    #[test]
    fn restarts_at_offset_zero() {
        let mut assembler = FrameAssembler::new(4);
        assert!(assembler.feed(&chunk(0, &[9, 9])).is_ok());
        assert!(assembler.feed(&chunk(0, &[1, 2, 3, 4])).is_ok());
        assert_eq!(assembler.feed(&[CMD_SHOW]), Ok(FrameAction::Show));
        assert_eq!(assembler.frame(), &[1, 2, 3, 4]);
    }

    #[test]
    fn clear_drops_the_staged_bytes() {
        let mut assembler = FrameAssembler::new(4);
        assert!(assembler.feed(&chunk(0, &[1, 2])).is_ok());
        assert_eq!(assembler.feed(&[CMD_CLEAR]), Ok(FrameAction::Clear));
        assert!(assembler.feed(&[CMD_SHOW]).is_err());
    }

    // -- JSON commands ----------------------------------------------------

    /// Encode like the app does, so the decoder is tested against real
    /// base64 rather than a hand-written constant.
    fn encode_base64(data: &[u8]) -> String {
        const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for triple in data.chunks(3) {
            let mut accumulator = 0u32;
            for index in 0..3 {
                accumulator = accumulator << 8 | u32::from(triple.get(index).copied().unwrap_or(0));
            }
            for index in 0..4 {
                if index <= triple.len() {
                    out.push(ALPHABET[(accumulator >> (18 - 6 * index) & 0x3F) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    #[test]
    fn base64_round_trips() {
        for data in [
            vec![],
            vec![0u8],
            vec![1u8, 2],
            vec![1u8, 2, 3],
            vec![0xFFu8; 1024],
        ] {
            assert_eq!(decode_base64(&encode_base64(&data)), Some(data.clone()));
        }
        // Outside the alphabet, or a length that cannot be base64.
        assert_eq!(decode_base64("not base64!!"), None);
        assert_eq!(decode_base64("AAAAA"), None);
        assert_eq!(decode_base64("=AAA"), None);
    }

    #[test]
    fn json_commands_are_acknowledged() {
        let mut display = EmulatedDisplay;
        let reply = handle_json_command(&mut display, r#"{"id": 7, "clear": true}"#);
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["ok"], true);

        let image = encode_base64(&vec![0u8; FRAME_SIZE]);
        let reply =
            handle_json_command(&mut display, &format!(r#"{{"id": 8, "image": "{image}"}}"#));
        assert_eq!(reply["id"], 8);
        assert_eq!(reply["ok"], true);
    }

    #[test]
    fn bad_json_commands_are_rejected_with_an_id() {
        let mut display = EmulatedDisplay;
        let short = encode_base64(&vec![0u8; 10]);
        for (message, want_id) in [
            ("not json".to_string(), json!(null)),
            (r#"{"clear": true}"#.into(), json!(null)), // no id
            (r#"{"id": "7", "clear": true}"#.into(), json!(null)), // id is not an integer
            (r#"{"id": 9}"#.into(), json!(9)),          // no action
            (r#"{"id": 10, "image": "not base64!!"}"#.into(), json!(10)),
            (format!(r#"{{"id": 11, "image": "{short}"}}"#), json!(11)),
        ] {
            let reply = handle_json_command(&mut display, &message);
            assert_eq!(reply["ok"], false, "{message} must be rejected");
            assert_eq!(reply["id"], want_id, "{message}");
            assert!(reply.contains_key("error"), "{message} must carry an error");
        }
    }
}
