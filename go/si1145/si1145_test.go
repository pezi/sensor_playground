// The chip reports the UV index multiplied by 100, and the visible/IR/UV
// result registers are 16-bit little-endian — both must match the Python
// node's decoding.
package main

import (
	"math"
	"testing"
)

func TestUVIndexScaling(t *testing.T) {
	cases := []struct {
		raw  uint16
		want float64
	}{
		{0, 0.0},
		{50, 0.5},       // rounds to 0.5 in the payload
		{123, 1.23},     // the payload rounds this to 1.2
		{800, 8.0},      // "very high" on the UV index scale
		{65535, 655.35}, // full scale; far beyond any real UV index
	}
	for _, c := range cases {
		if got := uvIndex(c.raw); math.Abs(got-c.want) > 1e-9 {
			t.Fatalf("uvIndex(%d) = %v, want %v", c.raw, got, c.want)
		}
	}
}

func TestUVIndexPayloadRounding(t *testing.T) {
	// The REST payload reports one decimal, like the Python node.
	if got := round1(uvIndex(123)); got != 1.2 {
		t.Fatalf("round1(uvIndex(123)) = %v, want 1.2", got)
	}
	if got := round1(uvIndex(475)); got != 4.8 {
		t.Fatalf("round1(uvIndex(475)) = %v, want 4.8", got)
	}
}
