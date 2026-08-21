// Unit tests for the portable SCD30 driver math: the Sensirion CRC-8
// (datasheet example 0xBE 0xEF -> 0x92), the per-word CRC frame parsing
// and the big-endian word-pair IEEE-754 float decoding, checked against
// the example measurement frame from the SCD30 interface description
// (439.09 ppm CO2, 27.2 °C, 48.8 %RH).
package main

import (
	"math"
	"testing"
)

// The read-measurement example frame from the Sensirion SCD30 interface
// description: CO2 0x43DB8C2E, temperature 0x41D9E7FF, humidity 0x42433A1B.
var scd30ExampleFrame = []byte{
	0x43, 0xDB, 0xCB, 0x8C, 0x2E, 0x8F, // CO2 = 439.095 ppm
	0x41, 0xD9, 0x70, 0xE7, 0xFF, 0xF5, // temperature = 27.238 °C
	0x42, 0x43, 0xBF, 0x3A, 0x1B, 0x74, // humidity = 48.806 %RH
}

func TestCRC8KnownVectors(t *testing.T) {
	cases := []struct {
		data []byte
		want uint8
	}{
		{[]byte{0xBE, 0xEF}, 0x92}, // Sensirion datasheet example
		{[]byte{0x00, 0x00}, 0x81}, // start-periodic argument (pressure 0)
		{[]byte{0x00, 0x02}, 0xE3}, // set-interval argument (2 s)
		{[]byte{0x43, 0xDB}, 0xCB},
		{[]byte{0x8C, 0x2E}, 0x8F},
	}
	for _, c := range cases {
		if got := scd30CRC8(c.data); got != c.want {
			t.Fatalf("crc8(% x) = 0x%02x, want 0x%02x", c.data, got, c.want)
		}
	}
}

func TestParseWords(t *testing.T) {
	words, err := parseSCD30Words(scd30ExampleFrame)
	if err != nil {
		t.Fatalf("valid frame rejected: %v", err)
	}
	if len(words) != 6 || words[0] != 0x43DB || words[1] != 0x8C2E {
		t.Fatalf("parsed words = %04x, want [43db 8c2e ...]", words)
	}

	corrupt := append([]byte(nil), scd30ExampleFrame...)
	corrupt[5] ^= 0x01
	if _, err := parseSCD30Words(corrupt); err == nil {
		t.Fatal("corrupt CRC accepted")
	}
	if _, err := parseSCD30Words(scd30ExampleFrame[:4]); err == nil {
		t.Fatal("short frame accepted")
	}
}

func TestFloatDecoding(t *testing.T) {
	// The words are combined MSW-first into a big-endian IEEE-754 single.
	if got := decodeSCD30Float(0x3F80, 0x0000); got != 1.0 {
		t.Fatalf("decodeFloat(0x3F80, 0x0000) = %v, want 1.0", got)
	}
	if got := decodeSCD30Float(0x0000, 0x0000); got != 0.0 {
		t.Fatalf("decodeFloat(0, 0) = %v, want 0.0", got)
	}

	co2, temperature, humidity, err := decodeSCD30Measurement(scd30ExampleFrame)
	if err != nil {
		t.Fatalf("valid measurement rejected: %v", err)
	}
	if math.Abs(co2-439.09515380859375) > 1e-9 {
		t.Fatalf("co2 = %v, want 439.095...", co2)
	}
	if math.Abs(temperature-27.238279342651367) > 1e-9 {
		t.Fatalf("temperature = %v, want 27.238...", temperature)
	}
	if math.Abs(humidity-48.80674362182617) > 1e-9 {
		t.Fatalf("humidity = %v, want 48.806...", humidity)
	}
}
