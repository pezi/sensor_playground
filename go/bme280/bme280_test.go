// Differential test: the compensation formulas are checked against golden
// values generated with the RPi.bme280 Python driver's double-precision
// formulas (the library the Python node uses), over a synthetic
// calibration set and 200 random ADC inputs.
package main

import (
	"encoding/json"
	"math"
	"os"
	"testing"
)

type goldenCase struct {
	RawT int64   `json:"raw_t"`
	RawP int64   `json:"raw_p"`
	RawH int64   `json:"raw_h"`
	T    float64 `json:"t"`
	P    float64 `json:"p"`
	H    float64 `json:"h"`
}

type goldens struct {
	Cal   map[string]float64 `json:"cal"`
	Cases []goldenCase       `json:"cases"`
}

func TestCompensationAgainstReferenceGoldens(t *testing.T) {
	data, err := os.ReadFile("testdata/compensation_goldens.json")
	if err != nil {
		t.Fatalf("reading goldens: %v", err)
	}
	var g goldens
	if err := json.Unmarshal(data, &g); err != nil {
		t.Fatalf("parsing goldens: %v", err)
	}

	c := bme280Calibration{
		digT1: g.Cal["T1"], digT2: g.Cal["T2"], digT3: g.Cal["T3"],
		digP1: g.Cal["P1"], digP2: g.Cal["P2"], digP3: g.Cal["P3"],
		digP4: g.Cal["P4"], digP5: g.Cal["P5"], digP6: g.Cal["P6"],
		digP7: g.Cal["P7"], digP8: g.Cal["P8"], digP9: g.Cal["P9"],
		digH1: g.Cal["H1"], digH2: g.Cal["H2"], digH3: g.Cal["H3"],
		digH4: g.Cal["H4"], digH5: g.Cal["H5"], digH6: g.Cal["H6"],
	}

	for i, gc := range g.Cases {
		temp, press, hum := compensateBME280(&c, gc.RawT, gc.RawP, gc.RawH)
		if math.Abs(temp-gc.T) > 1e-9 {
			t.Fatalf("case %d: temperature %v != golden %v", i, temp, gc.T)
		}
		if math.Abs(press-gc.P) > 1e-9 {
			t.Fatalf("case %d: pressure %v != golden %v", i, press, gc.P)
		}
		if math.Abs(hum-gc.H) > 1e-9 {
			t.Fatalf("case %d: humidity %v != golden %v", i, hum, gc.H)
		}
	}
}

// The calibration parser must assemble the shared H4/H5 nibbles from signed
// byte reads exactly as RPi.bme280 does.
func TestCalibrationH4H5SignedAssembly(t *testing.T) {
	c1 := make([]byte, 24)
	// c2: H2 lsb/msb, H3, 0xE4, 0xE5, 0xE6, H6
	c2 := []byte{0x78, 0x01, 0x00, 0x11, 0xC8, 0x1E, 0x1E}
	cal := parseBME280Calibration(c1, 75, c2)
	// e4=0x11(17), e5=0xC8(-56 signed), e6=0x1E(30)
	// H4 = 17<<4 | (-56 & 0x0F) = 272 | 8 = 280
	// H5 = ((-56 >> 4) & 0x0F) | 30<<4 = ((-4) & 0xF)=12 | 480 = 492
	if cal.digH4 != 280 {
		t.Fatalf("digH4 = %v, want 280", cal.digH4)
	}
	if cal.digH5 != 492 {
		t.Fatalf("digH5 = %v, want 492", cal.digH5)
	}
}
