// Sensor layer: real BME680 and emulated variant, both feeding the same
// IAQ calculation. Mirrors python/bme680/sensor_node.py.
package main

import (
	"math"
	"math/rand"
	"sync"
	"time"
)

const (
	gasBurnIn        = 50
	humidityBaseline = 40.0
	humidityWeight   = 0.25
)

// SensorData is one reading in REST units (temperature °C, humidity %RH,
// pressure hPa), already rounded like the Python node.
type SensorData struct {
	Temperature float64
	Humidity    float64
	Pressure    float64
	IAQ         int
}

type Sensor interface {
	Name() string
	// Read returns the current reading, or nil when the sensor read failed.
	Read() *SensorData
}

// ReadFull returns the REST payload for s, or nil on a failed read.
func ReadFull(s Sensor) map[string]any {
	d := s.Read()
	if d == nil {
		return nil
	}
	return map[string]any{
		"temperature": d.Temperature,
		"humidity":    d.Humidity,
		"pressure":    d.Pressure,
		"iaq":         d.IAQ,
	}
}

// ReadDiscovery returns the short-key discovery payload for s, or an
// empty map on a failed read.
func ReadDiscovery(s Sensor) map[string]any {
	d := s.Read()
	if d == nil {
		return map[string]any{}
	}
	return map[string]any{
		"temp":  d.Temperature,
		"hum":   d.Humidity,
		"press": d.Pressure,
		"iaq":   d.IAQ,
	}
}

func round1(x float64) float64 { return math.Round(x*10) / 10 }
func round2(x float64) float64 { return math.Round(x*100) / 100 }

// iaqState implements the rolling-baseline IAQ score ported from the
// dart_periphery BME680 driver (same algorithm as the Python/ESP32 nodes):
// a 50-reading gas-resistance window (pre-filled with zeros, so the score
// stabilizes only after ~50 readings), gas weighted 75%, humidity 25%.
type iaqState struct {
	gasData [gasBurnIn]int64
	next    int
	lastIAQ int
}

func (s *iaqState) calculate(gasResistance int64, humidity float64) int {
	s.gasData[s.next] = gasResistance
	s.next = (s.next + 1) % gasBurnIn

	var sum int64
	for _, v := range s.gasData {
		sum += v
	}
	gasBaseline := math.Round(float64(sum) / gasBurnIn)

	gasOffset := gasBaseline - float64(gasResistance)
	humOffset := humidity - humidityBaseline

	var humScore float64
	if humOffset > 0 {
		humScore = (100.0 - humidityBaseline - humOffset) / (100.0 - humidityBaseline) * (humidityWeight * 100.0)
	} else {
		humScore = (humidityBaseline + humOffset) / humidityBaseline * (humidityWeight * 100.0)
	}

	gasWeight := 100.0 - humidityWeight*100.0
	var gasScore float64
	if gasOffset > 0 {
		if gasBaseline == 0 {
			return s.lastIAQ // matches the Python ZeroDivisionError fallback
		}
		gasScore = float64(gasResistance) / gasBaseline * gasWeight
	} else {
		gasScore = gasWeight
	}

	s.lastIAQ = int(math.Round(humScore + gasScore))
	return s.lastIAQ
}

func (s *iaqState) current() int { return s.lastIAQ }

// -- Real sensor -------------------------------------------------------------

type BME680Sensor struct {
	mu  sync.Mutex
	dev interface{ Read() *Readings }
	iaq iaqState
}

func NewBME680Sensor(i2cBus int) (*BME680Sensor, error) {
	dev, err := NewBME680(i2cBus)
	if err != nil {
		return nil, err
	}
	return &BME680Sensor{dev: dev}, nil
}

func (s *BME680Sensor) Name() string { return "BME680" }

func (s *BME680Sensor) Read() *SensorData {
	s.mu.Lock()
	defer s.mu.Unlock()

	r := s.dev.Read()
	if r == nil {
		return nil
	}
	iaq := s.iaq.current()
	if r.GasValid && r.HeatStable {
		iaq = s.iaq.calculate(int64(r.GasResistance), r.Humidity)
	}
	return &SensorData{
		Temperature: round1(r.Temperature),
		Humidity:    round1(r.Humidity),
		Pressure:    round2(r.Pressure),
		IAQ:         iaq,
	}
}

// -- Emulated sensor ---------------------------------------------------------

// EmulatedBME680Sensor generates plausible readings without hardware: slow
// sines for temperature/humidity/pressure and a bounded random walk around
// 120 kOhm for the gas resistance, scored with the real IAQ algorithm.
type EmulatedBME680Sensor struct {
	mu  sync.Mutex
	iaq iaqState
	gas float64
}

func NewEmulatedBME680Sensor() *EmulatedBME680Sensor {
	return &EmulatedBME680Sensor{gas: 120000.0}
}

func (s *EmulatedBME680Sensor) Name() string { return "BME680" }

func (s *EmulatedBME680Sensor) Read() *SensorData {
	s.mu.Lock()
	defer s.mu.Unlock()

	t := float64(time.Now().UnixNano()) / 1e9
	s.gas = math.Min(math.Max(s.gas+uniform(-2000, 2000), 20000.0), 500000.0)
	humidity := 45.0 + 8.0*math.Sin(t/97.0) + uniform(-0.5, 0.5)
	return &SensorData{
		Temperature: round1(22.0 + 2.0*math.Sin(t/60.0) + uniform(-0.1, 0.1)),
		Humidity:    round1(humidity),
		Pressure:    round2(1013.0 + 3.0*math.Sin(t/300.0) + uniform(-0.2, 0.2)),
		IAQ:         s.iaq.calculate(int64(s.gas), humidity),
	}
}

func uniform(lo, hi float64) float64 { return lo + rand.Float64()*(hi-lo) }
