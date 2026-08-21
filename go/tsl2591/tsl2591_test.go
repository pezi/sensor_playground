// The lux equation must match the Python node's (the Adafruit library's
// two-approximation formula), including the integration-time-dependent
// saturation limit. The expected values below were produced by running the
// reference Python implementation on the same counts.
package main

import (
	"math"
	"testing"
)

func TestCalculateLux(t *testing.T) {
	cases := []struct {
		channel0, channel1 uint16
		integration        uint8
		again              float64
		want               float64
	}{
		{5400, 1500, 2, 25.0, 159.936},              // 300 ms, med gain — the defaults
		{200, 60, 0, 1.0, 414.5280000000001},        // 100 ms, low gain — bright sun
		{37000, 12000, 1, 428.0, 82.55327102803739}, // 200 ms, high gain
		{100, 0, 5, 9876.0, 0.00688537869582827},    // 600 ms, max gain — near darkness
	}
	for _, c := range cases {
		got, ok := calculateLux(c.channel0, c.channel1, c.integration, c.again)
		if !ok {
			t.Fatalf("calculateLux(%d,%d,%d,%v) reported saturation", c.channel0, c.channel1, c.integration, c.again)
		}
		if math.Abs(got-c.want) > 1e-9 {
			t.Fatalf("calculateLux(%d,%d,%d,%v) = %v, want %v",
				c.channel0, c.channel1, c.integration, c.again, got, c.want)
		}
	}
}

// At the shortest integration time the ADC only counts to 0x8FFF, so the
// same count means saturation at 100 ms but not at 300 ms.
func TestCalculateLuxSaturation(t *testing.T) {
	if _, ok := calculateLux(0x8FFF, 100, 0, 1.0); ok {
		t.Fatal("0x8FFF on channel 0 must saturate at 100 ms")
	}
	lux, ok := calculateLux(0x8FFF, 100, 2, 25.0)
	if !ok || math.Abs(lux-1996.4256) > 1e-9 {
		t.Fatalf("0x8FFF at 300 ms = %v (ok=%v), want 1996.4256", lux, ok)
	}
	if _, ok := calculateLux(1000, 0xFFFF, 2, 25.0); ok {
		t.Fatal("a saturated infrared channel must be rejected too")
	}
}

func TestGainIndexFor(t *testing.T) {
	for want, name := range []string{"low", "med", "high", "max"} {
		got, err := gainIndexFor(name)
		if err != nil || got != want {
			t.Fatalf("gainIndexFor(%q) = (%d, %v), want (%d, nil)", name, got, err, want)
		}
	}
	if _, err := gainIndexFor("medium"); err == nil {
		t.Fatal("an unknown gain name must be rejected")
	}
}
