// EEPROM record encode/decode tests — the same vectors as the Node.js and
// Rust ports, mirroring the Python node's behavior (magic 'S' 'P' + u16
// big-endian length + UTF-8 text, an absent magic reading as empty).
package main

import (
	"bytes"
	"testing"
)

func TestEncodeRecord(t *testing.T) {
	cases := []struct {
		name string
		text string
		want []byte
	}{
		{"ascii", "Hi", []byte{'S', 'P', 0x00, 0x02, 'H', 'i'}},
		{"empty", "", []byte{'S', 'P', 0x00, 0x00}},
		// "äöü" is 6 UTF-8 bytes, not 3 characters.
		{"multi-byte utf8", "äöü", []byte{'S', 'P', 0x00, 0x06, 0xC3, 0xA4, 0xC3, 0xB6, 0xC3, 0xBC}},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got := encodeRecord([]byte(tc.text))
			if !bytes.Equal(got, tc.want) {
				t.Errorf("encodeRecord(%q) = % X, want % X", tc.text, got, tc.want)
			}
		})
	}
}

// A length above 255 must land in the high byte of the header.
func TestEncodeRecordLongLength(t *testing.T) {
	text := bytes.Repeat([]byte("a"), 300)
	record := encodeRecord(text)
	if !bytes.Equal(record[:4], []byte{'S', 'P', 0x01, 0x2C}) {
		t.Errorf("header = % X, want 53 50 01 2C", record[:4])
	}
	if len(record) != headerSize+300 {
		t.Errorf("len = %d, want %d", len(record), headerSize+300)
	}
}

func TestRecordLength(t *testing.T) {
	cases := []struct {
		name   string
		header []byte
		want   int
	}{
		{"text", []byte{'S', 'P', 0x00, 0x05}, 5},
		{"empty text", []byte{'S', 'P', 0x00, 0x00}, 0},
		{"max length", []byte{'S', 'P', 0x02, 0x00}, textMaxBytes},
		// A factory-fresh chip is all 0xFF and holds no record.
		{"factory fresh", []byte{0xFF, 0xFF, 0xFF, 0xFF}, -1},
		{"blank zeros", []byte{0x00, 0x00, 0x00, 0x00}, -1},
		{"wrong magic", []byte{'X', 'P', 0x00, 0x02}, -1},
		// An implausible length reads as no record rather than as garbage.
		{"length over the maximum", []byte{'S', 'P', 0x02, 0x01}, -1},
		{"short header", []byte{'S', 'P', 0x00}, -1},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := recordLength(tc.header); got != tc.want {
				t.Errorf("recordLength(% X) = %d, want %d", tc.header, got, tc.want)
			}
		})
	}
}

func TestDecodeRecord(t *testing.T) {
	cases := []struct {
		name   string
		record []byte
		want   string
	}{
		{"ascii", []byte{'S', 'P', 0x00, 0x02, 'H', 'i'}, "Hi"},
		{"empty text", []byte{'S', 'P', 0x00, 0x00}, ""},
		{"factory fresh", bytes.Repeat([]byte{0xFF}, 16), ""},
		{"wrong magic", []byte{'N', 'O', 0x00, 0x02, 'H', 'i'}, ""},
		{"length over the maximum", []byte{'S', 'P', 0xFF, 0xFF, 'H', 'i'}, ""},
		// Only the declared length is text; trailing bytes are ignored.
		{"trailing bytes ignored", []byte{'S', 'P', 0x00, 0x02, 'H', 'i', 'X', 'Y'}, "Hi"},
		{
			"multi-byte utf8",
			[]byte{'S', 'P', 0x00, 0x08, 0xE2, 0x98, 0x95, 0xC3, 0xA4, 0xF0, 0x9F, 0x8E},
			"☕ä���", // the emoji is cut off by its declared length
		},
		// Each invalid byte becomes one U+FFFD, like Python's errors="replace".
		{"invalid utf8", []byte{'S', 'P', 0x00, 0x03, 'a', 0xFF, 'b'}, "a�b"},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := decodeRecord(tc.record); got != tc.want {
				t.Errorf("decodeRecord(% X) = %q, want %q", tc.record, got, tc.want)
			}
		})
	}
}

// encode -> decode is the identity for every text the node accepts.
func TestRoundTrip(t *testing.T) {
	for _, text := range []string{"", "Hello EEPROM", "äöü ☕", string(bytes.Repeat([]byte("x"), textMaxBytes))} {
		if got := decodeRecord(encodeRecord([]byte(text))); got != text {
			t.Errorf("round trip of %d bytes = %q", len(text), got)
		}
	}
}

// The emulation stores and returns the text just like the chip does.
func TestEmulatedEeprom(t *testing.T) {
	eeprom := NewEmulatedEeprom()
	if text, _ := eeprom.ReadText(); text != "Hello from the emulated EEPROM" {
		t.Errorf("initial text = %q", text)
	}
	if err := eeprom.WriteText([]byte("äöü")); err != nil {
		t.Fatal(err)
	}
	if text, _ := eeprom.ReadText(); text != "äöü" {
		t.Errorf("stored text = %q, want %q", text, "äöü")
	}
}

// A write past the text region is refused; the stored text stays put.
func TestControllerRejectsOversizeWrite(t *testing.T) {
	eeprom := NewEmulatedEeprom()
	controller := NewEepromController(eeprom)
	controller.Write([]byte("Hello"))
	controller.Write(bytes.Repeat([]byte("x"), textMaxBytes+1))
	if got := controller.Text(); got != "Hello" {
		t.Errorf("text after the rejected write = %q, want %q", got, "Hello")
	}
	// Even the rejected write publishes a fresh read-back.
	if !controller.TakePending() {
		t.Error("the rejected write left nothing pending")
	}
}
