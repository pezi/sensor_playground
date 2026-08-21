// Scaling tests: the MPU9250 accelerometer and AK8963 magnetometer
// conversions against hand-computed values at the configured full
// scales, and the BMP280 integer compensation against golden values
// generated with the Python node's own BMP280 class.
package main

import (
	"encoding/json"
	"math"
	"os"
	"testing"
)

func TestAccelConversionAt8G(t *testing.T) {
	// Big-endian raw counts: 16384 -> 4 g, -8192 -> -2 g, 8192 -> 2 g.
	x, y, z := convertAccel([]byte{0x40, 0x00, 0xE0, 0x00, 0x20, 0x00})
	if x != 4.0 || y != -2.0 || z != 2.0 {
		t.Fatalf("accel = (%v, %v, %v), want (4, -2, 2)", x, y, z)
	}
}

func TestMagConversionAt16Bit(t *testing.T) {
	// Little-endian raw counts with the 4912/32760 µT-per-count scale:
	// 3276 -> 491.2 µT, -3276 -> -491.2 µT, 32760 -> 4912 µT — each
	// multiplied by its factory sensitivity coefficient.
	magCal := [3]float64{1.0, 1.0, 0.5}
	x, y, z := convertMag([]byte{0xCC, 0x0C, 0x34, 0xF3, 0xF8, 0x7F, 0x00}, magCal)
	if math.Abs(x-491.2) > 1e-9 || math.Abs(y+491.2) > 1e-9 || math.Abs(z-2456.0) > 1e-9 {
		t.Fatalf("mag = (%v, %v, %v), want (491.2, -491.2, 2456)", x, y, z)
	}
}

func TestMagOverflowYieldsZeros(t *testing.T) {
	// A set ST2 overflow bit (0x08) discards the sample, like the
	// mpu9250-jmdev reference driver.
	magCal := [3]float64{1.0, 1.0, 1.0}
	x, y, z := convertMag([]byte{0xCC, 0x0C, 0x34, 0xF3, 0xF8, 0x7F, 0x08}, magCal)
	if x != 0 || y != 0 || z != 0 {
		t.Fatalf("mag = (%v, %v, %v), want zeros on overflow", x, y, z)
	}
}

func TestAnglesFromLevelBoard(t *testing.T) {
	// A level board pointing magnetic north-east: no roll/pitch, 1 g,
	// heading 45°.
	roll, pitch, heading, gforce := anglesFrom(0, 0, 1, 10, 10)
	if roll != 0 || pitch != 0 {
		t.Fatalf("roll/pitch = %v/%v, want 0/0", roll, pitch)
	}
	if math.Abs(heading-45.0) > 1e-9 {
		t.Fatalf("heading = %v, want 45", heading)
	}
	if gforce != 1.0 {
		t.Fatalf("gforce = %v, want 1", gforce)
	}
	// Negative atan2 results normalize into [0, 360) like Python's %.
	_, _, heading, _ = anglesFrom(0, 0, 1, 10, -10)
	if math.Abs(heading-315.0) > 1e-9 {
		t.Fatalf("heading = %v, want 315", heading)
	}
}

type bmpGoldenCase struct {
	AdcT int64   `json:"adc_t"`
	AdcP int64   `json:"adc_p"`
	T    float64 `json:"t"`
	P    float64 `json:"p"`
}

type bmpGoldens struct {
	Cal   map[string]int64 `json:"cal"`
	Cases []bmpGoldenCase  `json:"cases"`
}

func TestBMP280CompensationAgainstPythonGoldens(t *testing.T) {
	data, err := os.ReadFile("testdata/bmp280_goldens.json")
	if err != nil {
		t.Fatalf("reading goldens: %v", err)
	}
	var g bmpGoldens
	if err := json.Unmarshal(data, &g); err != nil {
		t.Fatalf("parsing goldens: %v", err)
	}

	c := bmp280Calibration{
		digT1: g.Cal["t1"], digT2: g.Cal["t2"], digT3: g.Cal["t3"],
		digP1: g.Cal["p1"], digP2: g.Cal["p2"], digP3: g.Cal["p3"],
		digP4: g.Cal["p4"], digP5: g.Cal["p5"], digP6: g.Cal["p6"],
		digP7: g.Cal["p7"], digP8: g.Cal["p8"], digP9: g.Cal["p9"],
	}

	for i, gc := range g.Cases {
		temp, press := compensateBMP280(&c, gc.AdcT, gc.AdcP)
		if math.Abs(temp-gc.T) > 1e-9 {
			t.Fatalf("case %d: temperature %v != golden %v", i, temp, gc.T)
		}
		if math.Abs(press-gc.P) > 1e-9 {
			t.Fatalf("case %d: pressure %v != golden %v", i, press, gc.P)
		}
	}
}
