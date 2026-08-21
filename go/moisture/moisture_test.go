package main

import "testing"

// The linear calibration mapping: adc_dry -> 0 %, adc_wet -> 100 %,
// clamped outside the calibrated span (the probe's output falls as the
// soil gets wetter).
func TestPercent(t *testing.T) {
	cases := []struct {
		raw, dry, wet, want int
	}{
		{2600, 2600, 1100, 0},   // dry calibration point
		{1100, 2600, 1100, 100}, // wet calibration point
		{1850, 2600, 1100, 50},  // midpoint
		{1820, 2600, 1100, 52},  // rounds to nearest percent
		{3000, 2600, 1100, 0},   // drier than dry -> clamps to 0
		{500, 2600, 1100, 100},  // wetter than wet -> clamps to 100
		{650, 650, 275, 0},      // 10-bit calibration points
		{275, 650, 275, 100},
	}
	for _, c := range cases {
		if got := percent(c.raw, c.dry, c.wet); got != c.want {
			t.Errorf("percent(%d, %d, %d) = %d, want %d", c.raw, c.dry, c.wet, got, c.want)
		}
	}
}
