// The CRC-8 and response parsing are checked against the SGP30 datasheet
// example (0xBEEF -> 0x92, section 6.6) and vectors generated with the
// Pimoroni sgp30-python reference driver.
package main

import "testing"

func TestSGP30CRC(t *testing.T) {
	cases := []struct {
		word uint16
		crc  uint8
	}{
		{0x0000, 0x81},
		{0xBEEF, 0x92}, // datasheet example
		{0x1234, 0x37},
		{0x0190, 0x4C}, // 400 ppm, the warm-up eCO2 value
		{0x8000, 0xA2},
		{0xFFFF, 0xAC},
	}
	for _, c := range cases {
		if got := sgp30CRC(c.word); got != c.crc {
			t.Fatalf("sgp30CRC(0x%04X) = 0x%02X, want 0x%02X", c.word, got, c.crc)
		}
	}
}

func TestParseSGP30Words(t *testing.T) {
	// A valid measure_air_quality warm-up response: 400 ppm, 0 ppb.
	words, err := parseSGP30Words([]byte{0x01, 0x90, 0x4C, 0x00, 0x00, 0x81})
	if err != nil {
		t.Fatalf("parseSGP30Words: %v", err)
	}
	if len(words) != 2 || words[0] != 400 || words[1] != 0 {
		t.Fatalf("parseSGP30Words = %v, want [400 0]", words)
	}

	// A corrupted CRC must be rejected.
	if _, err := parseSGP30Words([]byte{0x01, 0x90, 0x4D}); err == nil {
		t.Fatal("parseSGP30Words accepted an invalid CRC")
	}
}
