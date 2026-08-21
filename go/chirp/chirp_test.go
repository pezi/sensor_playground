package main

import "testing"

// The linear calibration mapping: cap_dry -> 0 %, cap_wet -> 100 %,
// clamped outside the calibrated span (the raw capacitance rises with
// moisture, so wet > dry).
func TestMoisturePercent(t *testing.T) {
	cases := []struct {
		capacitance, dry, wet, want int
	}{
		{290, 290, 520, 0},   // dry calibration point
		{520, 290, 520, 100}, // wet calibration point
		{405, 290, 520, 50},  // midpoint
		{350, 290, 520, 26},  // rounds to nearest percent
		{200, 290, 520, 0},   // drier than dry -> clamps to 0
		{700, 290, 520, 100}, // wetter than wet -> clamps to 100
		{300, 250, 600, 14},  // other calibration points
		{600, 250, 600, 100},
	}
	for _, c := range cases {
		if got := moisturePercent(c.capacitance, c.dry, c.wet); got != c.want {
			t.Errorf("moisturePercent(%d, %d, %d) = %d, want %d",
				c.capacitance, c.dry, c.wet, got, c.want)
		}
	}
}

// The chip's registers are big-endian 16-bit words.
func TestDecodeWord(t *testing.T) {
	cases := []struct {
		hi, lo byte
		want   uint16
	}{
		{0x00, 0x00, 0},
		{0x01, 0x2c, 300},
		{0x02, 0x08, 520},
		{0xff, 0x9c, 0xff9c},
		{0xff, 0xff, 65535},
	}
	for _, c := range cases {
		if got := decodeWord(c.hi, c.lo); got != c.want {
			t.Errorf("decodeWord(0x%02x, 0x%02x) = %d, want %d", c.hi, c.lo, got, c.want)
		}
	}
}

// The temperature register is a signed 16-bit value in tenths of a degree
// (two's complement, like the Python node's `raw -= 0x10000`).
func TestDecodeTemperature(t *testing.T) {
	cases := []struct {
		word uint16
		want float64
	}{
		{0x0000, 0.0},
		{0x0001, 0.1},
		{0x00d5, 21.3},
		{0x0d80, 345.6},
		{0x7fff, 3276.7}, // largest positive value
		{0xffff, -0.1},   // -1 -> -0.1 °C
		{0xff9c, -10.0},
		{0xfec4, -31.6},
		{0x8000, -3276.8}, // most negative value
	}
	for _, c := range cases {
		if got := decodeTemperature(c.word); got != c.want {
			t.Errorf("decodeTemperature(0x%04x) = %v, want %v", c.word, got, c.want)
		}
	}
}

// The chip counts a phototransistor discharge *up* in darkness, so the
// node inverts the raw value into brightness counts.
func TestLightCounts(t *testing.T) {
	cases := []struct {
		raw  uint16
		want int
	}{
		{0, 65535},     // brightest
		{65535, 0},     // darkest
		{20000, 45535}, // mid-scale emulation value
	}
	for _, c := range cases {
		if got := lightCounts(c.raw); got != c.want {
			t.Errorf("lightCounts(%d) = %d, want %d", c.raw, got, c.want)
		}
	}
}
