// The conversion formulas (datasheet V5: 14-bit temperature at 3.3V,
// 12-bit humidity polynomial + temperature compensation) are checked at
// exact raw values, including the 0..100 %RH clamp. There is no CRC to
// test: like the Python node, the driver ends the transfer before the
// CRC byte.
package main

import (
	"math"
	"testing"
)

func near(a, b float64) bool { return math.Abs(a-b) < 1e-9 }

// T = -39.66 + 0.01 * raw.
func TestTemperatureConversion(t *testing.T) {
	cases := []struct {
		raw  uint16
		want float64
	}{
		{0, -39.66},
		{3966, 0.0},
		{6566, 26.0},
	}
	for _, c := range cases {
		if got := sht1xTemperature(c.raw); !near(got, c.want) {
			t.Fatalf("temperature(%d) = %v, want %v", c.raw, got, c.want)
		}
	}
}

// RH = C1 + C2*raw + C3*raw² plus (T - 25)(T1 + T2*raw) compensation,
// clamped to the physical 0..100 %RH range.
func TestHumidityConversion(t *testing.T) {
	// At 25 °C the compensation term vanishes: the pure polynomial.
	if got := sht1xHumidity(1500, 25.0); !near(got, 49.413325) {
		t.Fatalf("humidity(1500, 25.0) = %v, want 49.413325", got)
	}
	// 10 °C above the reference adds 10 * (T1 + T2*1500) = 1.3.
	if got := sht1xHumidity(1500, 35.0); !near(got, 50.713325) {
		t.Fatalf("humidity(1500, 35.0) = %v, want 50.713325", got)
	}
	// The polynomial is negative near raw 0 and >100 at high raw values.
	if got := sht1xHumidity(0, 25.0); got != 0.0 {
		t.Fatalf("humidity(0, 25.0) = %v, want clamp to 0.0", got)
	}
	if got := sht1xHumidity(3500, 25.0); got != 100.0 {
		t.Fatalf("humidity(3500, 25.0) = %v, want clamp to 100.0", got)
	}
}
