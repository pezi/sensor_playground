// The colour maths must match the Python node's: the library's
// gamma-corrected RGB bytes and the DN40 lux / colour temperature
// algorithm. The expected values below were produced by running the
// reference Python implementation on the same raw counts.
package main

import (
	"math"
	"testing"
)

// The node's profile: 154 ms requested -> 64 cycles -> 153.6 ms, at 4x gain.
const (
	testIntegrationMs = 153.6
	testGain          = 4.0
)

func TestColorRGBBytes(t *testing.T) {
	cases := []struct {
		r, g, b, c   uint16
		wantR, wantG int
		wantB        int
	}{
		{1000, 1200, 900, 3500, 11, 17, 8},
		{5000, 4200, 3000, 12000, 28, 18, 8},
		{300, 400, 350, 900, 16, 33, 23},
		// Clear at zero is complete darkness: black, not a divide by zero.
		{0, 0, 0, 0, 0, 0, 0},
	}
	for _, c := range cases {
		r, g, b := colorRGBBytes(c.r, c.g, c.b, c.c)
		if r != c.wantR || g != c.wantG || b != c.wantB {
			t.Fatalf("colorRGBBytes(%d,%d,%d,%d) = (%d,%d,%d), want (%d,%d,%d)",
				c.r, c.g, c.b, c.c, r, g, b, c.wantR, c.wantG, c.wantB)
		}
	}
}

// A fully lit channel saturates at 255 rather than overflowing past it.
func TestColorRGBBytesClampsToByte(t *testing.T) {
	r, g, b := colorRGBBytes(65535, 65535, 65535, 65535)
	if r != 255 || g != 255 || b != 255 {
		t.Fatalf("colorRGBBytes at full scale = (%d,%d,%d), want (255,255,255)", r, g, b)
	}
}

func TestTemperatureAndLuxDN40(t *testing.T) {
	cases := []struct {
		r, g, b, c uint16
		wantLux    float64
		wantCT     float64
	}{
		{1000, 1200, 900, 3500, 472.46744791666663, 4820.0},
		{5000, 4200, 3000, 12000, 1755.2539062499998, 3645.8979591836733},
		{300, 400, 350, 900, 117.81412760416667, 6047.666666666667},
		// All channels dark: no light and the bare CT offset.
		{0, 0, 0, 0, 0.0, 1391.0},
	}
	for _, c := range cases {
		lux, colorTemperature, ok := temperatureAndLuxDN40(c.r, c.g, c.b, c.c, testIntegrationMs, testGain)
		if !ok {
			t.Fatalf("temperatureAndLuxDN40(%d,%d,%d,%d) reported saturation", c.r, c.g, c.b, c.c)
		}
		if math.Abs(lux-c.wantLux) > 1e-9 {
			t.Fatalf("lux(%d,%d,%d,%d) = %v, want %v", c.r, c.g, c.b, c.c, lux, c.wantLux)
		}
		if math.Abs(colorTemperature-c.wantCT) > 1e-9 {
			t.Fatalf("colorTemperature(%d,%d,%d,%d) = %v, want %v",
				c.r, c.g, c.b, c.c, colorTemperature, c.wantCT)
		}
	}
}

// A saturated clear channel says nothing about the colour, so the sample is
// rejected instead of being reported as a very bright reading.
func TestTemperatureAndLuxDN40RejectsSaturation(t *testing.T) {
	if _, _, ok := temperatureAndLuxDN40(60000, 60000, 60000, 65535, testIntegrationMs, testGain); ok {
		t.Fatal("a saturated clear channel must be rejected")
	}
	// Below 150 ms the saturation limit drops by a quarter (DN40 3.7): at
	// 100.8 ms the limit is 1024*42*0.75 = 32256 counts.
	if _, _, ok := temperatureAndLuxDN40(9000, 9000, 9000, 32256, 100.8, testGain); ok {
		t.Fatal("the ripple-reduced saturation limit must be applied below 150 ms")
	}
	lux, _, ok := temperatureAndLuxDN40(1000, 1200, 900, 3500, 100.8, testGain)
	if !ok || math.Abs(lux-719.9503968253969) > 1e-9 {
		t.Fatalf("lux at 100.8 ms = %v (ok=%v), want 719.9503968253969", lux, ok)
	}
}
