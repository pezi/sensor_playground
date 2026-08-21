// The raw RAM word → °C conversion must match the Python node's decoding:
// raw * 0.02 K - 273.15, with bit 15 marking an error rather than a
// temperature.
package main

import (
	"math"
	"testing"
)

func TestDecodeMLX90615Temp(t *testing.T) {
	cases := []struct {
		raw  uint16
		want float64
	}{
		{15000, 26.85},    // 15000 * 0.02 K = 300.00 K
		{14683, 20.51},    // room temperature
		{0x0000, -273.15}, // 0 K — what an empty bus reads back
		{0x7FFF, 382.19},  // largest value without the error flag
	}
	for _, c := range cases {
		got, valid := decodeMLX90615Temp(c.raw)
		if !valid {
			t.Fatalf("decodeMLX90615Temp(%#04x) reported an error flag", c.raw)
		}
		if math.Abs(got-c.want) > 1e-9 {
			t.Fatalf("decodeMLX90615Temp(%#04x) = %v, want %v", c.raw, got, c.want)
		}
	}
}

func TestDecodeMLX90615ErrorFlag(t *testing.T) {
	// Bit 15 set: the sensor reports an error, not a temperature.
	for _, raw := range []uint16{0x8000, 0xFFFF, 0xBAF6} {
		if _, valid := decodeMLX90615Temp(raw); valid {
			t.Fatalf("decodeMLX90615Temp(%#04x) must be invalid (bit 15 set)", raw)
		}
	}
}
