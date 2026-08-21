// The bitmap transposition and the command handling must match the Python
// node's. The expected values below were produced by running the reference
// Python implementation on the same input.
package main

import (
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"testing"
)

func TestToNativeFormatSinglePixel(t *testing.T) {
	// The top-left pixel is the MSB of the first byte in the app's format,
	// and the LSB of the first column byte in the panel's.
	data := make([]byte, frameSize)
	data[0] = 0x80
	native := toNativeFormat(data)
	if native[0] != 0x01 {
		t.Fatalf("native[0] = %#02x, want 0x01", native[0])
	}
	for index, value := range native[1:] {
		if value != 0 {
			t.Fatalf("native[%d] = %#02x, want 0 (only one pixel is lit)", index+1, value)
		}
	}
}

func TestToNativeFormatBottomRightOfFirstPage(t *testing.T) {
	// Row 7, x = 127: the last row of page 0 (the MSB of a column byte) at
	// the last column, which is the last byte of the first page.
	data := make([]byte, frameSize)
	data[7*rowBytes+15] = 0x01
	native := toNativeFormat(data)
	if native[127] != 0x80 {
		t.Fatalf("native[127] = %#02x, want 0x80", native[127])
	}
}

func TestToNativeFormatUniformPatterns(t *testing.T) {
	white := make([]byte, frameSize)
	for i := range white {
		white[i] = 0xFF
	}
	for index, value := range toNativeFormat(white) {
		if value != 0xFF {
			t.Fatalf("all-white native[%d] = %#02x, want 0xff", index, value)
		}
	}

	// Every other row lit: each column byte holds eight vertical pixels, so
	// alternating rows become 0b01010101.
	stripes := make([]byte, frameSize)
	for row := 0; row < displayHeight; row += 2 {
		for i := 0; i < rowBytes; i++ {
			stripes[row*rowBytes+i] = 0xFF
		}
	}
	for index, value := range toNativeFormat(stripes) {
		if value != 0x55 {
			t.Fatalf("striped native[%d] = %#02x, want 0x55", index, value)
		}
	}
}

// A full pseudo-random frame, checked against the reference implementation
// by hash — the uniform patterns above cannot catch a transposition that
// scrambles bytes within a page.
func TestToNativeFormatMatchesReferenceFrame(t *testing.T) {
	data := make([]byte, frameSize)
	for i := range data {
		data[i] = byte((i*37 + 11) % 256)
	}
	digest := sha256.Sum256(toNativeFormat(data))
	const want = "c94eac32b46614437efa4e68d4493e213ab0d0a04e6b19ae532fae0cd92f64d3"
	if got := hex.EncodeToString(digest[:]); got != want {
		t.Fatalf("transposed frame sha256 = %s, want %s", got, want)
	}
}

// -- JSON commands --------------------------------------------------------

// countingDisplay records the calls the command handler makes.
type countingDisplay struct {
	cleared int
	shown   int
}

func (d *countingDisplay) Clear() error                 { d.cleared++; return nil }
func (d *countingDisplay) ShowBitmap(data []byte) error { d.shown++; return nil }

func TestHandleJSONCommandAcks(t *testing.T) {
	display := &countingDisplay{}

	reply := handleJSONCommand(display, []byte(`{"id": 7, "clear": true}`))
	if reply["ok"] != true || reply["id"] != 7 {
		t.Fatalf("clear ACK = %v, want id 7 ok true", reply)
	}
	if display.cleared != 1 {
		t.Fatalf("clear called %d times, want 1", display.cleared)
	}

	image := base64.StdEncoding.EncodeToString(make([]byte, frameSize))
	reply = handleJSONCommand(display, []byte(`{"id": 8, "image": "`+image+`"}`))
	if reply["ok"] != true || reply["id"] != 8 {
		t.Fatalf("image ACK = %v, want id 8 ok true", reply)
	}
	if display.shown != 1 {
		t.Fatalf("show called %d times, want 1", display.shown)
	}
}

func TestHandleJSONCommandNacks(t *testing.T) {
	display := &countingDisplay{}
	short := base64.StdEncoding.EncodeToString(make([]byte, 10))
	cases := []struct {
		message string
		wantID  any
	}{
		{`not json`, nil},
		{`{"clear": true}`, nil},                     // no id
		{`{"id": "7", "clear": true}`, nil},          // id is not an integer
		{`{"id": 9}`, 9},                             // no action
		{`{"id": 10, "image": "not base64!!"}`, 10},  // undecodable
		{`{"id": 11, "image": "` + short + `"}`, 11}, // wrong size
	}
	for _, c := range cases {
		reply := handleJSONCommand(display, []byte(c.message))
		if reply["ok"] != false {
			t.Fatalf("%s: ok = %v, want false", c.message, reply["ok"])
		}
		if reply["id"] != c.wantID {
			t.Fatalf("%s: id = %v, want %v", c.message, reply["id"], c.wantID)
		}
		if reply["error"] == nil || reply["error"] == "" {
			t.Fatalf("%s: a NACK must carry an error", c.message)
		}
	}
	// None of them reached the panel.
	if display.cleared != 0 || display.shown != 0 {
		t.Fatalf("rejected commands must not touch the display (%d clears, %d shows)",
			display.cleared, display.shown)
	}
}
