// Unit tests for the portable MMA7660 math: 6-bit two's-complement axis
// decoding, the 21.33 counts-per-g conversion and the roll/pitch/g-force
// formulas, mirrored from the Python node's smbus2 access.
package main

import (
	"math"
	"testing"
)

func approx(t *testing.T, name string, got, want float64) {
	t.Helper()
	if math.Abs(got-want) > 1e-9 {
		t.Errorf("%s: got %v, want %v", name, got, want)
	}
}

// 6-bit two's complement: 0..31 positive, 32..63 wrap to -32..-1.
func TestDecodeAxis(t *testing.T) {
	cases := []struct {
		raw  uint8
		want int
	}{
		{0, 0}, {1, 1}, {31, 31}, {32, -32}, {63, -1}, {43, -21},
	}
	for _, c := range cases {
		if got := decodeAxis(c.raw); got != c.want {
			t.Errorf("decodeAxis(%d): got %d, want %d", c.raw, got, c.want)
		}
	}
}

// Counts -> g with 21.33 counts per g.
func TestDecodeAxesGConversion(t *testing.T) {
	x, y, z, ok := decodeAxes([]byte{21, 63, 32})
	if !ok {
		t.Fatal("decodeAxes: unexpected alert")
	}
	approx(t, "x", x, 21/21.33)
	approx(t, "y", y, -1/21.33)
	approx(t, "z", z, -32/21.33)
}

// The alert bit (0x40) invalidates the whole block.
func TestDecodeAxesAlertBit(t *testing.T) {
	if _, _, _, ok := decodeAxes([]byte{0x40, 0, 21}); ok {
		t.Error("decodeAxes: alert bit not detected")
	}
	if _, _, _, ok := decodeAxes([]byte{0, 0x55, 21}); ok {
		t.Error("decodeAxes: alert bit in y not detected")
	}
}

func TestOrientation(t *testing.T) {
	// Flat board resting at 1 g: no roll, no pitch.
	roll, pitch, gforce := orientation(0, 0, 1.0)
	approx(t, "roll", roll, 0)
	approx(t, "pitch", pitch, 0)
	approx(t, "gforce", gforce, 1.0)

	// 45° roll: y and z pull equally.
	roll, pitch, _ = orientation(0, 0.7, 0.7)
	approx(t, "roll", roll, 45)
	approx(t, "pitch", pitch, 0)

	// Nose-down: gravity entirely on -x -> +90° pitch.
	_, pitch, gforce = orientation(-1.0, 0, 0)
	approx(t, "pitch", pitch, 90)
	approx(t, "gforce", gforce, 1.0)
}
