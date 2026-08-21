package main

import (
	"math"
	"testing"
	"time"
)

// The ratio -> concentration conversion (Nafis curve). Golden values from
// the Python node's formula: 1.1*r^3 - 3.8*r^2 + 520*r + 0.62.
func TestConcentrationCurve(t *testing.T) {
	cases := []struct {
		ratio float64
		want  float64
	}{
		{0, 0.62},         // clean window
		{1, 517.92},       // 1 % occupancy
		{2, 1034.22},      // 2 % occupancy
		{5.5, 2928.6825},  // mid-range, exercises the cubic term
		{100, 1114000.62}, // fully low window (stuck-low saturation)
	}
	for _, c := range cases {
		got := concentration(c.ratio)
		if math.Abs(got-c.want) > 1e-6 {
			t.Errorf("concentration(%v) = %v, want %v", c.ratio, got, c.want)
		}
	}
}

// Warm-up: before the first full 30-second window closes, read() returns
// nil (REST 503 / identity-only discovery reply).
func TestReadDuringWarmup(t *testing.T) {
	s := &ppd42ns{windowStart: time.Now()}
	if got := s.read(); got != nil {
		t.Errorf("read() during warm-up = %v, want nil", got)
	}
	if got := discoveryFor(s.read)(); len(got) != 0 {
		t.Errorf("discovery during warm-up = %v, want empty map", got)
	}
}

// A due window with zero accumulated low time reads as clean air: the
// curve's offset 0.62 rounds to 0.6.
func TestReadCleanWindow(t *testing.T) {
	s := &ppd42ns{windowStart: time.Now().Add(-31 * time.Second)}
	got := s.read()
	if got == nil {
		t.Fatal("read() after a full window = nil, want a payload")
	}
	if dust := got["dust"].(float64); dust != 0.6 {
		t.Errorf(`read()["dust"] = %v, want 0.6`, dust)
	}
}
