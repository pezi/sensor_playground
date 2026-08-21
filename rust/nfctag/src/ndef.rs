//! NDEF parsing for the Grove NFC Tag node — a faithful port of the Python
//! node's parse_ndef_area/_parse_ndef_record, including its permissive error
//! handling: anything that fails to parse is reported as a hex dump rather
//! than dropped, so the app always sees that *something* was written.

use common::Payload;
use serde_json::json;

/// Bytes of the EEPROM scanned for the NDEF message (CC + TLV area).
pub const SCAN_LENGTH: usize = 256;

/// Cap for hex dumps of unparseable payloads (bytes before hex encoding).
const DATA_HEX_CAP: usize = 64;

/// NFC Forum URI record prefix codes (the common subset; the same table
/// lives in the ESP32 sketch — keep them identical).
const URI_PREFIXES: [&str; 7] = [
    "",             // 0x00
    "http://www.",  // 0x01
    "https://www.", // 0x02
    "http://",      // 0x03
    "https://",     // 0x04
    "tel:",         // 0x05
    "mailto:",      // 0x06
];

/// One parsed tag content — the state this node pushes. `kind` is "text",
/// "uri", "data" or "empty"; only "empty" has no value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub kind: &'static str,
    pub value: String,
}

impl Content {
    fn new(kind: &'static str, value: impl Into<String>) -> Self {
        Self {
            kind,
            value: value.into(),
        }
    }

    /// The empty content of a blank or NDEF-less tag.
    fn empty() -> Self {
        Self::new("empty", "")
    }

    /// The JSON message pushed to clients, matching the Python node:
    /// {"kind": "text", "value": "..."} — "empty" carries no value key.
    pub fn payload(&self) -> Payload {
        let mut p = Payload::new();
        p.insert("kind".into(), json!(self.kind));
        if self.kind != "empty" {
            p.insert("value".into(), json!(self.value));
        }
        p
    }
}

/// Parses the scanned EEPROM area into a [`Content`].
///
/// The area starts with the Type 5 capability container (magic 0xE1/0xE2),
/// followed by a TLV stream in which 0x03 marks the NDEF message.
pub fn parse_ndef_area(data: &[u8]) -> Content {
    if data.len() < 4 || (data[0] != 0xE1 && data[0] != 0xE2) {
        if data.iter().all(|&b| b == 0x00 || b == 0xFF) {
            return Content::empty();
        }
        return data_content(data);
    }

    let mut offset = 4; // first byte after the 4-byte capability container
    while offset < data.len() {
        let tlv = data[offset];
        if tlv == 0x00 {
            // padding
            offset += 1;
            continue;
        }
        if tlv == 0xFE {
            // terminator: no NDEF TLV found
            return Content::empty();
        }
        if tlv != 0x03 {
            // unknown TLV: skip it (1-byte length format)
            if offset + 1 >= data.len() {
                return Content::empty();
            }
            offset += 2 + usize::from(data[offset + 1]);
            continue;
        }
        // NDEF message TLV: 1-byte length, or 0xFF + 2-byte big-endian.
        if offset + 1 >= data.len() {
            return Content::empty();
        }
        let mut length = usize::from(data[offset + 1]);
        offset += 2;
        if length == 0xFF {
            if offset + 2 > data.len() {
                return Content::empty();
            }
            length = usize::from(data[offset]) << 8 | usize::from(data[offset + 1]);
            offset += 2;
        }
        if length == 0 {
            return Content::empty();
        }
        let message = clamp(data, offset, offset.saturating_add(length));
        if message.len() < length {
            return data_content(message); // truncated by the scan window
        }
        return parse_ndef_record(message);
    }
    Content::empty()
}

/// Parses the first record of an NDEF message. An index the record's
/// declared lengths put outside the message is the Python node's
/// IndexError: the whole message is reported as a hex dump.
fn parse_ndef_record(message: &[u8]) -> Content {
    if message.len() < 2 {
        return data_content(message);
    }
    let flags = message[0];
    let tnf = flags & 0x07;
    let short_record = flags & 0x10 != 0;
    let has_id = flags & 0x08 != 0;
    let type_length = usize::from(message[1]);
    let mut offset = 2;
    let payload_length = if short_record {
        if offset >= message.len() {
            return data_content(message);
        }
        let n = usize::from(message[offset]);
        offset += 1;
        n
    } else {
        let n = be_uint(clamp(message, offset, offset + 4));
        offset += 4;
        n
    };
    let mut id_length = 0;
    if has_id {
        if offset >= message.len() {
            return data_content(message);
        }
        id_length = usize::from(message[offset]);
        offset += 1;
    }
    let record_type = clamp(message, offset, offset.saturating_add(type_length));
    offset = offset.saturating_add(type_length).saturating_add(id_length);
    let payload = clamp(message, offset, offset.saturating_add(payload_length));
    if payload.len() < payload_length {
        return data_content(payload);
    }

    if tnf == 0x01 && record_type == b"T" {
        if payload.is_empty() {
            return data_content(message);
        }
        let status = payload[0];
        let lang_length = usize::from(status & 0x3F);
        let text = clamp(payload, 1 + lang_length, payload.len());
        let value = if status & 0x80 != 0 {
            decode_utf16_replace(text)
        } else {
            decode_utf8_replace(text)
        };
        return Content::new("text", value);
    }
    if tnf == 0x01 && record_type == b"U" {
        if payload.is_empty() {
            return data_content(message);
        }
        let prefix = URI_PREFIXES
            .get(usize::from(payload[0]))
            .copied()
            .unwrap_or("");
        return Content::new(
            "uri",
            format!("{prefix}{}", decode_utf8_replace(&payload[1..])),
        );
    }
    data_content(payload)
}

/// Hex-dump fallback for content that is not a text or URI record.
fn data_content(data: &[u8]) -> Content {
    let capped = &data[..data.len().min(DATA_HEX_CAP)];
    let mut hex = String::with_capacity(capped.len() * 2);
    for byte in capped {
        hex.push_str(&format!("{byte:02X}"));
    }
    Content::new("data", hex)
}

/// Slices like Python: out-of-range bounds clamp instead of panicking.
fn clamp(b: &[u8], start: usize, end: usize) -> &[u8] {
    let start = start.min(b.len());
    let end = end.min(b.len());
    if start > end {
        return &[];
    }
    &b[start..end]
}

/// Python's int.from_bytes(b, "big") — tolerates short slices.
fn be_uint(b: &[u8]) -> usize {
    b.iter().fold(0usize, |acc, &x| acc << 8 | usize::from(x))
}

/// Decodes UTF-8 with U+FFFD for each invalid byte, like Python's
/// errors="replace".
fn decode_utf8_replace(bytes: &[u8]) -> String {
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

/// Decodes UTF-16 like Python's "utf-16" codec: a BOM selects the byte
/// order, without one little-endian is assumed; unpaired surrogates and a
/// trailing odd byte become U+FFFD.
fn decode_utf16_replace(bytes: &[u8]) -> String {
    let mut little_endian = true;
    let mut b = bytes;
    if b.len() >= 2 {
        if b[0] == 0xFF && b[1] == 0xFE {
            b = &b[2..];
        } else if b[0] == 0xFE && b[1] == 0xFF {
            little_endian = false;
            b = &b[2..];
        }
    }
    let units = b.chunks_exact(2).map(|c| {
        if little_endian {
            u16::from_le_bytes([c[0], c[1]])
        } else {
            u16::from_be_bytes([c[0], c[1]])
        }
    });
    let mut s: String = char::decode_utf16(units)
        .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect();
    if b.len() % 2 != 0 {
        s.push(char::REPLACEMENT_CHARACTER);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Type 5 capability container (magic 0xE1).
    const CC: [u8; 4] = [0xE1, 0x40, 0x40, 0x00];

    /// NDEF short record: well-known Text, "en", "Hello".
    const TEXT_HELLO: [u8; 12] = [
        0xD1, 0x01, 0x08, 0x54, 0x02, 0x65, 0x6E, 0x48, 0x65, 0x6C, 0x6C, 0x6F,
    ];

    fn area(tlvs: &[&[u8]]) -> Vec<u8> {
        let mut data = CC.to_vec();
        for tlv in tlvs {
            data.extend_from_slice(tlv);
        }
        data
    }

    fn content(kind: &'static str, value: &str) -> Content {
        Content::new(kind, value)
    }

    #[test]
    fn parses_ndef_area() {
        let cases: Vec<(&str, Vec<u8>, Content)> = vec![
            ("blank zeros", vec![0u8; SCAN_LENGTH], Content::empty()),
            ("blank ff", vec![0xFFu8; SCAN_LENGTH], Content::empty()),
            ("empty scan", Vec::new(), Content::empty()),
            (
                "no capability container",
                vec![0xDE, 0xAD, 0xBE, 0xEF],
                content("data", "DEADBEEF"),
            ),
            ("short garbage", vec![0x12], content("data", "12")),
            (
                "text record",
                area(&[&[0x03, 0x0C], &TEXT_HELLO, &[0xFE]]),
                content("text", "Hello"),
            ),
            (
                "uri record",
                area(&[
                    &[0x03, 0x0D, 0xD1, 0x01, 0x09, 0x55, 0x04],
                    b"seeed.cc",
                    &[0xFE],
                ]),
                content("uri", "https://seeed.cc"),
            ),
            (
                "utf16 text record",
                area(&[
                    &[
                        0x03, 0x0B, 0xD1, 0x01, 0x07, 0x54, 0x82, 0x65, 0x6E, 0x48, 0x00, 0x69,
                        0x00,
                    ],
                    &[0xFE],
                ]),
                content("text", "Hi"),
            ),
            (
                "three-byte tlv length",
                area(&[&[0x03, 0xFF, 0x00, 0x0C], &TEXT_HELLO, &[0xFE]]),
                content("text", "Hello"),
            ),
            (
                "padding and unknown tlv skipped",
                area(&[
                    &[0x00, 0x01, 0x02, 0xAA, 0xBB, 0x03, 0x0C],
                    &TEXT_HELLO,
                    &[0xFE],
                ]),
                content("text", "Hello"),
            ),
            (
                "unknown record type",
                area(&[&[0x03, 0x06, 0xD2, 0x01, 0x02, 0x78, 0xDE, 0xAD, 0xFE]]),
                content("data", "DEAD"),
            ),
            ("terminator only", area(&[&[0xFE]]), Content::empty()),
            (
                "zero-length ndef tlv",
                area(&[&[0x03, 0x00, 0xFE]]),
                Content::empty(),
            ),
            (
                "message truncated by scan window",
                area(&[&[0x03, 0x10, 0xD1, 0x01]]),
                content("data", "D101"),
            ),
            (
                "text record without payload",
                area(&[&[0x03, 0x04, 0xD1, 0x01, 0x00, 0x54, 0xFE]]),
                content("data", "D1010054"),
            ),
        ];

        for (name, data, want) in cases {
            assert_eq!(parse_ndef_area(&data), want, "{name}");
        }
    }

    #[test]
    fn data_hex_dump_is_capped() {
        assert_eq!(
            parse_ndef_area(&[0xABu8; 80]),
            content("data", &"AB".repeat(DATA_HEX_CAP))
        );
    }

    #[test]
    fn empty_payload_has_no_value_key() {
        assert_eq!(
            serde_json::to_string(&serde_json::Value::Object(Content::empty().payload())).unwrap(),
            r#"{"kind":"empty"}"#
        );
        assert_eq!(
            serde_json::to_string(&serde_json::Value::Object(
                content("text", "Hello").payload()
            ))
            .unwrap(),
            r#"{"kind":"text","value":"Hello"}"#
        );
    }
}
