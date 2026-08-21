// NDEF parsing for the Grove NFC Tag node — a faithful port of the Python
// node's parse_ndef_area/_parse_ndef_record, including its permissive error
// handling: anything that fails to parse is reported as a hex dump rather
// than dropped, so the app always sees that *something* was written.
package main

import (
	"encoding/hex"
	"strings"
	"unicode/utf16"
	"unicode/utf8"
)

const (
	// Bytes of the EEPROM scanned for the NDEF message (CC + TLV area).
	scanLength = 256

	// Cap for hex dumps of unparseable payloads (bytes before hex encoding).
	dataHexCap = 64
)

// NFC Forum URI record prefix codes (the common subset; the same table
// lives in the ESP32 sketch — keep them identical).
var uriPrefixes = map[byte]string{
	0x00: "",
	0x01: "http://www.",
	0x02: "https://www.",
	0x03: "http://",
	0x04: "https://",
	0x05: "tel:",
	0x06: "mailto:",
}

// Content is one parsed tag content — the state this node pushes.
// Kind is "text", "uri", "data" or "empty"; only "empty" has no value.
type Content struct {
	Kind  string
	Value string
}

// Payload is the JSON message pushed to clients, matching the Python node:
// {"kind": "text", "value": "..."} — the "empty" kind carries no value key.
func (c Content) Payload() map[string]any {
	if c.Kind == "empty" {
		return map[string]any{"kind": c.Kind}
	}
	return map[string]any{"kind": c.Kind, "value": c.Value}
}

// parseNdefArea parses the scanned EEPROM area into a Content.
//
// The area starts with the Type 5 capability container (magic 0xE1/0xE2),
// followed by a TLV stream in which 0x03 marks the NDEF message.
func parseNdefArea(data []byte) Content {
	if len(data) < 4 || (data[0] != 0xE1 && data[0] != 0xE2) {
		if allBlank(data) {
			return Content{Kind: "empty"}
		}
		return dataContent(data)
	}

	offset := 4 // first byte after the 4-byte capability container
	for offset < len(data) {
		tlv := data[offset]
		if tlv == 0x00 { // padding
			offset++
			continue
		}
		if tlv == 0xFE { // terminator: no NDEF TLV found
			return Content{Kind: "empty"}
		}
		if tlv != 0x03 { // unknown TLV: skip it (1-byte length format)
			if offset+1 >= len(data) {
				return Content{Kind: "empty"}
			}
			offset += 2 + int(data[offset+1])
			continue
		}
		// NDEF message TLV: 1-byte length, or 0xFF + 2-byte big-endian.
		if offset+1 >= len(data) {
			return Content{Kind: "empty"}
		}
		length := int(data[offset+1])
		offset += 2
		if length == 0xFF {
			if offset+2 > len(data) {
				return Content{Kind: "empty"}
			}
			length = int(data[offset])<<8 | int(data[offset+1])
			offset += 2
		}
		if length == 0 {
			return Content{Kind: "empty"}
		}
		message := clampSlice(data, offset, offset+length)
		if len(message) < length {
			return dataContent(message) // truncated by the scan window
		}
		return parseNdefRecord(message)
	}
	return Content{Kind: "empty"}
}

// parseNdefRecord parses the first record of an NDEF message. An index the
// record's declared lengths put outside the message is the Python node's
// IndexError: the whole message is reported as a hex dump.
func parseNdefRecord(message []byte) Content {
	if len(message) < 2 {
		return dataContent(message)
	}
	flags := message[0]
	tnf := flags & 0x07
	shortRecord := flags&0x10 != 0
	hasID := flags&0x08 != 0
	typeLength := int(message[1])
	offset := 2
	var payloadLength int
	if shortRecord {
		if offset >= len(message) {
			return dataContent(message)
		}
		payloadLength = int(message[offset])
		offset++
	} else {
		payloadLength = beUint(clampSlice(message, offset, offset+4))
		offset += 4
	}
	idLength := 0
	if hasID {
		if offset >= len(message) {
			return dataContent(message)
		}
		idLength = int(message[offset])
		offset++
	}
	recordType := clampSlice(message, offset, offset+typeLength)
	offset += typeLength + idLength
	payload := clampSlice(message, offset, offset+payloadLength)
	if len(payload) < payloadLength {
		return dataContent(payload)
	}

	if tnf == 0x01 && string(recordType) == "T" {
		if len(payload) == 0 {
			return dataContent(message)
		}
		status := payload[0]
		langLength := int(status & 0x3F)
		text := clampSlice(payload, 1+langLength, len(payload))
		if status&0x80 != 0 {
			return Content{Kind: "text", Value: decodeUTF16Replace(text)}
		}
		return Content{Kind: "text", Value: decodeUTF8Replace(text)}
	}
	if tnf == 0x01 && string(recordType) == "U" {
		if len(payload) == 0 {
			return dataContent(message)
		}
		rest := decodeUTF8Replace(payload[1:])
		return Content{Kind: "uri", Value: uriPrefixes[payload[0]] + rest}
	}
	return dataContent(payload)
}

// dataContent is the hex-dump fallback for content that is not a text or
// URI record.
func dataContent(data []byte) Content {
	if len(data) > dataHexCap {
		data = data[:dataHexCap]
	}
	return Content{Kind: "data", Value: strings.ToUpper(hex.EncodeToString(data))}
}

// allBlank reports whether every byte is 0x00 or 0xFF (erased EEPROM).
func allBlank(data []byte) bool {
	for _, b := range data {
		if b != 0x00 && b != 0xFF {
			return false
		}
	}
	return true
}

// clampSlice slices like Python: out-of-range bounds clamp instead of
// panicking.
func clampSlice(b []byte, start, end int) []byte {
	if start > len(b) {
		start = len(b)
	}
	if end > len(b) {
		end = len(b)
	}
	if start > end {
		return nil
	}
	return b[start:end]
}

// beUint is Python's int.from_bytes(b, "big") — tolerates short slices.
func beUint(b []byte) int {
	v := 0
	for _, x := range b {
		v = v<<8 | int(x)
	}
	return v
}

// decodeUTF8Replace decodes UTF-8 with U+FFFD for each invalid byte, like
// Python's errors="replace".
func decodeUTF8Replace(b []byte) string {
	var sb strings.Builder
	for len(b) > 0 {
		r, size := utf8.DecodeRune(b)
		if r == utf8.RuneError && size == 1 {
			sb.WriteRune(utf8.RuneError)
		} else {
			sb.WriteRune(r)
		}
		b = b[size:]
	}
	return sb.String()
}

// decodeUTF16Replace decodes UTF-16 like Python's "utf-16" codec: a BOM
// selects the byte order, without one little-endian is assumed; unpaired
// surrogates and a trailing odd byte become U+FFFD.
func decodeUTF16Replace(b []byte) string {
	littleEndian := true
	if len(b) >= 2 {
		if b[0] == 0xFF && b[1] == 0xFE {
			b = b[2:]
		} else if b[0] == 0xFE && b[1] == 0xFF {
			littleEndian = false
			b = b[2:]
		}
	}
	units := make([]uint16, 0, len(b)/2)
	for i := 0; i+1 < len(b); i += 2 {
		if littleEndian {
			units = append(units, uint16(b[i])|uint16(b[i+1])<<8)
		} else {
			units = append(units, uint16(b[i])<<8|uint16(b[i+1]))
		}
	}
	s := string(utf16.Decode(units))
	if len(b)%2 != 0 {
		s += string(utf8.RuneError)
	}
	return s
}
