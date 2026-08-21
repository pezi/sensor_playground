// The raw counts → payload derivation must match the Python node: lux from
// the green channel, the colour normalized against the brightest channel,
// and no colour at all in complete darkness.
package main

import "testing"

func TestDeriveReadingNormalizesAgainstBrightestChannel(t *testing.T) {
	reading := deriveReading(30000, 60000, 15000)
	// Red is the brightest channel, so it saturates at 255 and the others
	// are scaled against it.
	for key, want := range map[string]int{"red": 255, "green": 128, "blue": 64} {
		if got := reading[key]; got != want {
			t.Fatalf("%s = %v, want %v", key, got, want)
		}
	}
	// Green drives the illuminance: 30000 * 10000/65535 ≈ 4578 lux.
	if got := reading["lux"]; got != 4578 {
		t.Fatalf("lux = %v, want 4578", got)
	}
}

func TestDeriveReadingFullScale(t *testing.T) {
	reading := deriveReading(65535, 65535, 65535)
	if got := reading["lux"]; got != 10000 {
		t.Fatalf("lux = %v, want 10000 (top of the 10K range)", got)
	}
	for _, key := range []string{"red", "green", "blue"} {
		if got := reading[key]; got != 255 {
			t.Fatalf("%s = %v, want 255", key, got)
		}
	}
}

func TestDeriveReadingInDarknessHasNoColour(t *testing.T) {
	reading := deriveReading(0, 0, 0)
	if got := reading["lux"]; got != 0 {
		t.Fatalf("lux = %v, want 0", got)
	}
	for _, key := range []string{"red", "green", "blue"} {
		if _, present := reading[key]; present {
			t.Fatalf("%s must be absent in complete darkness, got %v", key, reading[key])
		}
	}
}
