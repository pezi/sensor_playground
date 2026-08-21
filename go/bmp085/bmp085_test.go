package main

import (
	"math"
	"testing"
)

// The worked example from the BMP085 datasheet (section 3.5): with these
// calibration constants, raw_t=27898 and raw_p=23843 at oversampling 0
// compensate to 15.0 °C and 69964 Pa.
func TestDatasheetWorkedExample(t *testing.T) {
	c := bmpCalibration{
		ac1: 408, ac2: -72, ac3: -14383, ac4: 32741, ac5: 32757, ac6: 23153,
		b1: 6190, b2: 4, mb: -32768, mc: -8711, md: 2868,
	}
	temperature, pressure := compensateBMP085(&c, 27898, 23843, 0)
	if temperature != 15.0 {
		t.Fatalf("temperature %v != 15.0", temperature)
	}
	if pressure != 69964 {
		t.Fatalf("pressure %v != 69964", pressure)
	}
}

func TestAltitudeAtSeaLevel(t *testing.T) {
	if a := altitudeFor(seaLevelPa); math.Abs(a) > 1e-9 {
		t.Fatalf("altitude at sea level pressure = %v, want 0", a)
	}
	// The datasheet's example: 69964 Pa is roughly 3000 m.
	if a := altitudeFor(69964); math.Abs(a-3021) > 30 {
		t.Fatalf("altitude for 69964 Pa = %v, want ~3021 m", a)
	}
}

func TestCalibrationRejectsBusGarbage(t *testing.T) {
	all := make([]byte, 22)
	if _, err := parseBMPCalibration(all); err == nil {
		t.Fatal("all-zero calibration accepted")
	}
	for i := range all {
		all[i] = 0xFF
	}
	if _, err := parseBMPCalibration(all); err == nil {
		t.Fatal("all-0xFF calibration accepted")
	}
}
