// Unit tests: RDM630 frame parsing and XOR checksum, fed byte-by-byte the
// way the UART delivers them.
package main

import "testing"

// feedAll runs a byte sequence through the parser and collects the tags.
func feedAll(p *FrameParser, data []byte) []string {
	var tags []string
	for _, b := range data {
		if tag, ok := p.Feed(b); ok {
			tags = append(tags, tag)
		}
	}
	return tags
}

// frame wraps 12 hex chars in STX/ETX.
func frame(chars string) []byte {
	f := []byte{stx}
	f = append(f, []byte(chars)...)
	return append(f, etx)
}

func TestParsesValidFrame(t *testing.T) {
	tags := feedAll(&FrameParser{}, frame("0F0024ADAB2D"))
	if len(tags) != 1 || tags[0] != "0F0024ADAB" {
		t.Fatalf("got %v, want [0F0024ADAB]", tags)
	}
}

func TestAcceptsLowercaseHex(t *testing.T) {
	tags := feedAll(&FrameParser{}, frame("0f0024adab2d"))
	if len(tags) != 1 || tags[0] != "0F0024ADAB" {
		t.Fatalf("got %v, want [0F0024ADAB]", tags)
	}
}

func TestRejectsBadChecksum(t *testing.T) {
	if tags := feedAll(&FrameParser{}, frame("0F0024ADAB2C")); len(tags) != 0 {
		t.Fatalf("got %v, want no tag", tags)
	}
}

func TestRejectsShortFrame(t *testing.T) {
	if tags := feedAll(&FrameParser{}, frame("0F0024ADAB")); len(tags) != 0 {
		t.Fatalf("got %v, want no tag", tags)
	}
}

func TestDiscardsOverflow(t *testing.T) {
	if tags := feedAll(&FrameParser{}, frame("0F0024ADAB2D00")); len(tags) != 0 {
		t.Fatalf("got %v, want no tag", tags)
	}
}

func TestResyncsOnSecondStx(t *testing.T) {
	data := append([]byte{stx, '0', 'F', '0', '0'}, frame("0F0024ADAB2D")...)
	tags := feedAll(&FrameParser{}, data)
	if len(tags) != 1 || tags[0] != "0F0024ADAB" {
		t.Fatalf("got %v, want [0F0024ADAB]", tags)
	}
}

func TestIgnoresNoiseAndRecovers(t *testing.T) {
	p := &FrameParser{}
	// Noise outside a frame is ignored (including a stray ETX)...
	if tags := feedAll(p, []byte{0x00, 'A', etx}); len(tags) != 0 {
		t.Fatalf("got %v, want no tag", tags)
	}
	// ...non-hex noise mid-frame aborts the frame, and parsing recovers
	// on the next one.
	data := append([]byte{stx, '0', 'F', '$'}, frame("1000C0FFEEC1")...)
	tags := feedAll(p, data)
	if len(tags) != 1 || tags[0] != "1000C0FFEE" {
		t.Fatalf("got %v, want [1000C0FFEE]", tags)
	}
}

func TestChecksumMatchesTagPool(t *testing.T) {
	for _, valid := range []string{"0F0024ADAB2D", "0A0031B2C44D", "03004F19AAFF", "1000C0FFEEC1"} {
		if !checksumOK([]byte(valid)) {
			t.Errorf("checksumOK(%q) = false, want true", valid)
		}
	}
	if checksumOK([]byte("0F0024ADAB2C")) {
		t.Error(`checksumOK("0F0024ADAB2C") = true, want false`)
	}
}
