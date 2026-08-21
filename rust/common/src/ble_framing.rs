//! MTU-safe framing for Sensor Playground BLE data notifications.
//! Port of python/common/ble_framing.py.
//!
//! Each packet is at most 20 bytes, which fits the payload available at
//! BLE's mandatory 23-byte ATT MTU. The Flutter client also accepts legacy
//! unframed JSON, so nodes and applications can be upgraded independently.

pub const FRAME_MARKER: u8 = 0x1E;
pub const FRAME_START: u8 = 0x01;
pub const FRAME_END: u8 = 0x02;
pub const FRAME_HEADER_LENGTH: usize = 4;
pub const FRAME_CONTENT_LENGTH: usize = 20 - FRAME_HEADER_LENGTH;

/// Ordered notification packets for `payload`.
pub fn frame_payload(payload: &[u8], message_id: u8) -> Vec<Vec<u8>> {
    if payload.is_empty() {
        return Vec::new();
    }
    let mut packets = Vec::new();
    for (chunk_index, chunk) in payload.chunks(FRAME_CONTENT_LENGTH).enumerate() {
        assert!(chunk_index <= 0xFF, "BLE payload needs more than 256 chunks");
        let mut flags = 0u8;
        if chunk_index == 0 {
            flags |= FRAME_START;
        }
        if (chunk_index + 1) * FRAME_CONTENT_LENGTH >= payload.len() {
            flags |= FRAME_END;
        }
        let mut packet = Vec::with_capacity(FRAME_HEADER_LENGTH + chunk.len());
        packet.extend_from_slice(&[FRAME_MARKER, message_id, chunk_index as u8, flags]);
        packet.extend_from_slice(chunk);
        packets.push(packet);
    }
    packets
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_chunk_has_both_flags() {
        let packets = frame_payload(b"{}", 7);
        assert_eq!(packets, vec![vec![0x1E, 7, 0, 0x03, b'{', b'}']]);
    }

    #[test]
    fn multi_chunk_flags_and_order() {
        let payload: Vec<u8> = (0..40).collect(); // 3 chunks: 16 + 16 + 8
        let packets = frame_payload(&payload, 200);
        assert_eq!(packets.len(), 3);
        assert_eq!(&packets[0][..4], &[0x1E, 200, 0, 0x01]);
        assert_eq!(&packets[1][..4], &[0x1E, 200, 1, 0x00]);
        assert_eq!(&packets[2][..4], &[0x1E, 200, 2, 0x02]);
        assert_eq!(packets[2].len(), 4 + 8);
        let reassembled: Vec<u8> = packets.iter().flat_map(|p| p[4..].to_vec()).collect();
        assert_eq!(reassembled, payload);
    }

    #[test]
    fn empty_payload_no_packets() {
        assert!(frame_payload(b"", 0).is_empty());
    }
}
