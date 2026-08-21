// Unit tests for the portable MPU6050 math: big-endian 16-bit decoding,
// the 16384 LSB-per-g conversion (incl. negative values), the
// die-temperature formula and the roll/pitch/g-force formulas, mirrored
// from the Python node's smbus2 access.
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

// Big-endian two's complement: 0x8000 wraps to -32768, 0xFFFF to -1.
func TestS16BigEndian(t *testing.T) {
	cases := []struct {
		raw  []byte
		want int
	}{
		{[]byte{0x00, 0x00}, 0},
		{[]byte{0x00, 0x01}, 1},
		{[]byte{0x7F, 0xFF}, 32767},
		{[]byte{0x80, 0x00}, -32768},
		{[]byte{0xFF, 0xFF}, -1},
		{[]byte{0xC0, 0x00}, -16384},
	}
	for _, c := range cases {
		if got := s16be(c.raw, 0); got != c.want {
			t.Errorf("s16be(% x): got %d, want %d", c.raw, got, c.want)
		}
	}
}

// Raw -> g with 16384 LSB per g, and raw/340 + 36.53 for the die
// temperature; the trailing gyroscope words are ignored.
func TestDecodeAccelTemp(t *testing.T) {
	raw := []byte{
		0x20, 0x00, // ax =   8192 -> 0.5 g
		0xC0, 0x00, // ay = -16384 -> -1.0 g
		0x40, 0x00, // az =  16384 -> 1.0 g
		0xF9, 0x5C, // temp = -1700 -> -5.0 + 36.53 = 31.53 °C
		0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, // gyroscope (unused)
	}
	x, y, z, temperature := decodeAccelTemp(raw)
	approx(t, "x", x, 0.5)
	approx(t, "y", y, -1.0)
	approx(t, "z", z, 1.0)
	approx(t, "temperature", temperature, 31.53)
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
