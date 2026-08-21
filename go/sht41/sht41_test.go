// Unit tests for the portable SHT41 driver math: the Sensirion CRC-8
// (datasheet example 0xBE 0xEF -> 0x92), frame parsing and the datasheet
// conversion formulas (0x6666 = exactly 40% full scale, so temperature
// 25.0 °C and humidity 44.0 %RH fall out exactly).
package main

import (
	"math"
	"testing"
)

func TestCRC8KnownVectors(t *testing.T) {
	cases := []struct {
		data []byte
		want uint8
	}{
		{[]byte{0xBE, 0xEF}, 0x92}, // SHT4x datasheet example
		{[]byte{0x00, 0x00}, 0x81},
		{[]byte{0x66, 0x66}, 0x93},
		{[]byte{0x80, 0x00}, 0xA2},
		{[]byte{0xFF, 0xFF}, 0xAC},
	}
	for _, c := range cases {
		if got := sht41CRC8(c.data); got != c.want {
			t.Fatalf("crc8(% x) = 0x%02x, want 0x%02x", c.data, got, c.want)
		}
	}
}

func TestParseFrame(t *testing.T) {
	tempRaw, humRaw, err := parseSHT41Frame([]byte{0x66, 0x66, 0x93, 0x80, 0x00, 0xA2})
	if err != nil {
		t.Fatalf("valid frame rejected: %v", err)
	}
	if tempRaw != 0x6666 || humRaw != 0x8000 {
		t.Fatalf("parsed raw = 0x%04x/0x%04x, want 0x6666/0x8000", tempRaw, humRaw)
	}

	if _, _, err := parseSHT41Frame([]byte{0x66, 0x66, 0x94, 0x80, 0x00, 0xA2}); err == nil {
		t.Fatal("corrupt temperature CRC accepted")
	}
	if _, _, err := parseSHT41Frame([]byte{0x66, 0x66, 0x93, 0x80, 0x00, 0xA3}); err == nil {
		t.Fatal("corrupt humidity CRC accepted")
	}
	if _, _, err := parseSHT41Frame([]byte{0x66, 0x66, 0x93}); err == nil {
		t.Fatal("short frame accepted")
	}
}

func TestConversions(t *testing.T) {
	cases := []struct {
		tempRaw, humRaw uint16
		temp, hum       float64
	}{
		{0x0000, 0x0000, -45.0, 0.0},   // humidity -6 clamped to 0
		{0xFFFF, 0xFFFF, 130.0, 100.0}, // humidity 119 clamped to 100
		{0x6666, 0x6666, 25.0, 44.0},   // 0x6666/0xFFFF = 0.4 exactly
	}
	for _, c := range cases {
		temp, hum := convertSHT41(c.tempRaw, c.humRaw)
		if math.Abs(temp-c.temp) > 1e-9 {
			t.Fatalf("temperature(0x%04x) = %v, want %v", c.tempRaw, temp, c.temp)
		}
		if math.Abs(hum-c.hum) > 1e-9 {
			t.Fatalf("humidity(0x%04x) = %v, want %v", c.humRaw, hum, c.hum)
		}
	}

	// An unclamped midpoint, against the formula evaluated independently.
	temp, hum := convertSHT41(0x8000, 0x8000)
	if math.Abs(temp-42.50133516441596) > 1e-9 {
		t.Fatalf("temperature(0x8000) = %v", temp)
	}
	if math.Abs(hum-56.50095368886854) > 1e-9 {
		t.Fatalf("humidity(0x8000) = %v", hum)
	}
}
