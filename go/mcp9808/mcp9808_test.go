// The raw-word → °C conversion must match the Python node's decoding:
// 12-bit magnitude in 1/16 °C, bit 12 sign (two's complement), alert
// flag bits 15..13 ignored.
package main

import "testing"

func TestDecodeMCP9808Temp(t *testing.T) {
	cases := []struct {
		word uint16
		want float64
	}{
		{0x0000, 0.0},
		{0x0001, 0.0625},   // one LSB
		{0x0194, 25.25},    // MCP9808 datasheet example (+25.25 °C)
		{0x0FFF, 255.9375}, // largest positive magnitude
		{0x1FFF, -0.0625},  // -1 LSB (two's complement)
		{0x1FE8, -1.5},
		{0x1E70, -25.0},
		{0x1000, -256.0},
		{0xE194, 25.25}, // alert flag bits 15..13 must be masked off
	}
	for _, c := range cases {
		if got := decodeMCP9808Temp(c.word); got != c.want {
			t.Fatalf("decodeMCP9808Temp(%#04x) = %v, want %v", c.word, got, c.want)
		}
	}
}
