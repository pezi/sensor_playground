// Differential test: temperature, pressure, humidity and heater values use
// Pimoroni reference goldens; gas values correct its unsigned decoding of
// Bosch's signed range-switch-error calibration field.
package main

import (
	"encoding/json"
	"math"
	"os"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

type goldenCase struct {
	AdcT  int64   `json:"adc_t"`
	AdcP  int64   `json:"adc_p"`
	AdcH  int64   `json:"adc_h"`
	AdcG  int64   `json:"adc_g"`
	Range uint8   `json:"range"`
	T     int64   `json:"t"`
	TFine int64   `json:"tfine"`
	P     int64   `json:"p"`
	H     int64   `json:"h"`
	G     float64 `json:"g"`
	Heat  int64   `json:"heat"`
}

type goldens struct {
	Cal          []byte       `json:"cal"`
	HeatRangeReg uint8        `json:"heat_range_reg"`
	HeatVal      uint8        `json:"heat_val"`
	SwErrReg     uint8        `json:"sw_err_reg"`
	Cases        []goldenCase `json:"cases"`
}

func TestCompensationAgainstCorrectedGoldens(t *testing.T) {
	data, err := os.ReadFile("testdata/compensation_goldens.json")
	if err != nil {
		t.Fatalf("reading goldens: %v", err)
	}
	var g goldens
	if err := json.Unmarshal(data, &g); err != nil {
		t.Fatalf("parsing goldens: %v", err)
	}

	s := &BME680{}
	s.calib = parseCalibration(g.Cal, g.HeatRangeReg, g.HeatVal, g.SwErrReg)
	if s.calib.rangeSwErr != -4 {
		t.Fatalf("signed range switch error %d != -4", s.calib.rangeSwErr)
	}

	for i, c := range g.Cases {
		temp := s.calcTemperature(c.AdcT)
		if temp != c.T {
			t.Fatalf("case %d: temperature %d != golden %d", i, temp, c.T)
		}
		if s.calib.tFine != c.TFine {
			t.Fatalf("case %d: t_fine %d != golden %d", i, s.calib.tFine, c.TFine)
		}
		if p := s.calcPressure(c.AdcP); p != c.P {
			t.Fatalf("case %d: pressure %d != golden %d", i, p, c.P)
		}
		if h := s.calcHumidity(c.AdcH); h != c.H {
			t.Fatalf("case %d: humidity %d != golden %d", i, h, c.H)
		}
		if gas := s.calcGasResistanceLow(c.AdcG, c.Range); math.Abs(gas-c.G) > 1e-6 {
			t.Fatalf("case %d: gas %v != golden %v", i, gas, c.G)
		}
		s.ambient = temp
		if hr := int64(s.calcHeaterResistance(320)); hr != c.Heat {
			t.Fatalf("case %d: heater resistance %d != golden %d", i, hr, c.Heat)
		}
	}
}

func TestInvalidGasDoesNotAdvanceIAQBaseline(t *testing.T) {
	var state iaqState
	first := state.calculate(120000, 45.0)
	next := state.next

	if got := state.current(); got != first {
		t.Fatalf("current IAQ %d != last calculated IAQ %d", got, first)
	}
	if state.next != next {
		t.Fatalf("reading current IAQ advanced baseline from %d to %d", next, state.next)
	}
}

type concurrencyProbe struct {
	active    atomic.Int32
	maxActive atomic.Int32
}

func (p *concurrencyProbe) Read() *Readings {
	active := p.active.Add(1)
	for {
		maximum := p.maxActive.Load()
		if active <= maximum || p.maxActive.CompareAndSwap(maximum, active) {
			break
		}
	}
	time.Sleep(time.Millisecond)
	p.active.Add(-1)
	return &Readings{
		Temperature:   22,
		Pressure:      1013,
		Humidity:      45,
		GasResistance: 120000,
		GasValid:      true,
		HeatStable:    true,
	}
}

func TestSensorSerializesReads(t *testing.T) {
	probe := &concurrencyProbe{}
	sensor := &BME680Sensor{dev: probe}
	var wait sync.WaitGroup
	for range 8 {
		wait.Add(1)
		go func() {
			defer wait.Done()
			if sensor.Read() == nil {
				t.Error("concurrent read unexpectedly failed")
			}
		}()
	}
	wait.Wait()
	if maximum := probe.maxActive.Load(); maximum != 1 {
		t.Fatalf("maximum concurrent driver reads %d != 1", maximum)
	}
}

type fixedReading struct{ value *Readings }

func (f fixedReading) Read() *Readings { return f.value }

func TestSensorRejectsInvalidGasForIAQ(t *testing.T) {
	sensor := &BME680Sensor{dev: fixedReading{&Readings{
		Temperature:   22,
		Pressure:      1013,
		Humidity:      45,
		GasResistance: 0,
		GasValid:      false,
		HeatStable:    true,
	}}}
	data := sensor.Read()
	if data == nil || data.IAQ != 0 {
		t.Fatalf("invalid gas returned IAQ data %#v", data)
	}
	if sensor.iaq.next != 0 {
		t.Fatalf("invalid gas advanced IAQ baseline to %d", sensor.iaq.next)
	}
}
