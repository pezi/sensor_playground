// The CRC-8 is checked against the SHT3x datasheet example
// (CRC(0xBEEF) = 0x92) and the conversion formulas against exact raw
// values from the datasheet formulas (§4.13).
package main

import (
	"math"
	"testing"
)

func TestCRCDatasheetExample(t *testing.T) {
	if crc := sht31CRC([]byte{0xBE, 0xEF}); crc != 0x92 {
		t.Fatalf("CRC(0xBEEF) = 0x%02x, want 0x92", crc)
	}
}

func TestConversionFormulas(t *testing.T) {
	cases := []struct {
		raw       uint16
		temp, hum float64
	}{
		{0x0000, -45.0, 0.0},
		{0xFFFF, 130.0, 100.0},
		{26214, 25.0, 40.0}, // 26214/65535 = 2/5
	}
	for _, c := range cases {
		if got := sht31Temperature(c.raw); math.Abs(got-c.temp) > 1e-9 {
			t.Fatalf("temperature(%d) = %v, want %v", c.raw, got, c.temp)
		}
		if got := sht31Humidity(c.raw); math.Abs(got-c.hum) > 1e-9 {
			t.Fatalf("humidity(%d) = %v, want %v", c.raw, got, c.hum)
		}
	}
}

func TestParseFrame(t *testing.T) {
	// temp raw 26214 (25.0 °C), hum raw 26214 (40.0 %RH), valid CRCs.
	frame := []byte{0x66, 0x66, sht31CRC([]byte{0x66, 0x66}), 0x66, 0x66, sht31CRC([]byte{0x66, 0x66})}
	temp, hum, err := parseSHT31Frame(frame)
	if err != nil {
		t.Fatalf("parseSHT31Frame: %v", err)
	}
	if math.Abs(temp-25.0) > 1e-9 || math.Abs(hum-40.0) > 1e-9 {
		t.Fatalf("parseSHT31Frame = (%v, %v), want (25.0, 40.0)", temp, hum)
	}

	bad := append([]byte(nil), frame...)
	bad[2] ^= 0xFF
	if _, _, err := parseSHT31Frame(bad); err == nil {
		t.Fatal("corrupted temperature CRC not detected")
	}
	bad = append([]byte(nil), frame...)
	bad[4] ^= 0x01
	if _, _, err := parseSHT31Frame(bad); err == nil {
		t.Fatal("corrupted humidity word not detected")
	}
}
