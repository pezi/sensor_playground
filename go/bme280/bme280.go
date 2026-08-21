// Compact BME280 driver, ported from the RPi.bme280 Python driver
// (https://github.com/rm-hull/bme280, MIT) — the library the Python node
// uses — so the readings match it: double-precision compensation formulas
// from Appendix A (8.1) of the BME280 datasheet, forced mode with x1
// oversampling.
package main

import (
	"fmt"
	"time"

	"sensorplayground/common"
)

const (
	bmeCalibration1 = 0x88 // dig_T*, dig_P* (24 bytes)
	bmeDigH1        = 0xA1
	bmeCalibration2 = 0xE1 // dig_H2..dig_H6 (7 bytes)
	bmeCtrlHum      = 0xF2
	bmeCtrlMeas     = 0xF4
	bmeData         = 0xF7 // press msb..hum lsb (8 bytes)

	bmeOversampling = 1 // x1, the RPi.bme280 default the Python node uses
	bmeForcedMode   = 1
)

type bme280Calibration struct {
	digT1, digT2, digT3                                           float64
	digP1, digP2, digP3, digP4, digP5, digP6, digP7, digP8, digP9 float64
	digH1, digH2, digH3, digH4, digH5, digH6                      float64
}

// BME280Readings is one forced-mode measurement.
type BME280Readings struct {
	Temperature float64 // °C
	Pressure    float64 // hPa
	Humidity    float64 // %RH
}

type BME280 struct {
	dev   *common.I2CDevice
	calib bme280Calibration
}

// NewBME280 opens the sensor on /dev/i2c-<bus> at 0x76 and loads its
// calibration EEPROM.
func NewBME280(bus int) (*BME280, error) {
	dev, err := common.OpenI2C(bus, 0x76)
	if err != nil {
		return nil, err
	}
	s := &BME280{dev: dev}
	if err := s.readCalibration(); err != nil {
		dev.Close()
		return nil, fmt.Errorf("reading BME280 calibration: %w", err)
	}
	return s, nil
}

func (s *BME280) Close() error { return s.dev.Close() }

func (s *BME280) readCalibration() error {
	c1, err := s.dev.ReadRegs(bmeCalibration1, 24)
	if err != nil {
		return err
	}
	h1, err := s.dev.ReadReg(bmeDigH1)
	if err != nil {
		return err
	}
	c2, err := s.dev.ReadRegs(bmeCalibration2, 7)
	if err != nil {
		return err
	}
	s.calib = parseBME280Calibration(c1, h1, c2)
	return nil
}

// parseBME280Calibration mirrors RPi.bme280's load_calibration_params:
// 16-bit words are little-endian; H4/H5 share a nibble, assembled from
// *signed* byte reads exactly as the reference does.
func parseBME280Calibration(c1 []byte, h1 uint8, c2 []byte) bme280Calibration {
	u16 := func(b []byte, i int) float64 { return float64(uint16(b[i]) | uint16(b[i+1])<<8) }
	s16 := func(b []byte, i int) float64 { return float64(int16(uint16(b[i]) | uint16(b[i+1])<<8)) }
	s8 := func(v uint8) int { return int(int8(v)) }

	e4, e5, e6 := s8(c2[3]), s8(c2[4]), s8(c2[5])
	return bme280Calibration{
		digT1: u16(c1, 0), digT2: s16(c1, 2), digT3: s16(c1, 4),
		digP1: u16(c1, 6), digP2: s16(c1, 8), digP3: s16(c1, 10),
		digP4: s16(c1, 12), digP5: s16(c1, 14), digP6: s16(c1, 16),
		digP7: s16(c1, 18), digP8: s16(c1, 20), digP9: s16(c1, 22),
		digH1: float64(h1),
		digH2: s16(c2, 0),
		digH3: float64(int8(c2[2])),
		digH4: float64(e4<<4 | (e5 & 0x0F)),
		digH5: float64(((e5 >> 4) & 0x0F) | e6<<4),
		digH6: float64(int8(c2[6])),
	}
}

// Read triggers a forced x1-oversampling measurement, waits the datasheet
// conversion time and returns compensated readings.
func (s *BME280) Read() (*BME280Readings, error) {
	if err := s.dev.WriteReg(bmeCtrlHum, bmeOversampling); err != nil {
		return nil, err
	}
	ctrl := uint8(bmeOversampling<<5 | bmeOversampling<<2 | bmeForcedMode)
	if err := s.dev.WriteReg(bmeCtrlMeas, ctrl); err != nil {
		return nil, err
	}
	// RPi.bme280's __calc_delay for x1/x1/x1, rounded up.
	time.Sleep(12 * time.Millisecond)

	block, err := s.dev.ReadRegs(bmeData, 8)
	if err != nil {
		return nil, err
	}
	rawP := (int64(block[0])<<16 | int64(block[1])<<8 | int64(block[2])) >> 4
	rawT := (int64(block[3])<<16 | int64(block[4])<<8 | int64(block[5])) >> 4
	rawH := int64(block[6])<<8 | int64(block[7])

	temperature, pressure, humidity := compensateBME280(&s.calib, rawT, rawP, rawH)
	return &BME280Readings{Temperature: temperature, Pressure: pressure, Humidity: humidity}, nil
}

func bme280TFine(c *bme280Calibration, t int64) float64 {
	tf := float64(t)
	v1 := (tf/16384.0 - c.digT1/1024.0) * c.digT2
	d := tf/131072.0 - c.digT1/8192.0
	v2 := d * d * c.digT3
	return v1 + v2
}

// compensateBME280 converts raw ADC values to (°C, hPa, %RH) with the
// double-precision formulas as transcribed in RPi.bme280.
func compensateBME280(c *bme280Calibration, rawT, rawP, rawH int64) (temperature, pressure, humidity float64) {
	tFine := bme280TFine(c, rawT)
	temperature = tFine / 5120.0

	// Humidity.
	res := tFine - 76800.0
	res = (float64(rawH) - (c.digH4*64.0 + c.digH5/16384.0*res)) *
		(c.digH2 / 65536.0 * (1.0 + c.digH6/67108864.0*res*(1.0+c.digH3/67108864.0*res)))
	res = res * (1.0 - c.digH1*res/524288.0)
	humidity = max(0.0, min(res, 100.0))

	// Pressure (Pa, then hPa).
	v1 := tFine/2.0 - 64000.0
	v2 := v1 * v1 * c.digP6 / 32768.0
	v2 = v2 + v1*c.digP5*2.0
	v2 = v2/4.0 + c.digP4*65536.0
	v1 = (c.digP3*v1*v1/524288.0 + c.digP2*v1) / 524288.0
	v1 = (1.0 + v1/32768.0) * c.digP1
	if v1 == 0 {
		return temperature, 0, humidity
	}
	p := 1048576.0 - float64(rawP)
	p = ((p - v2/4096.0) * 6250.0) / v1
	v1 = c.digP9 * p * p / 2147483648.0
	v2 = p * c.digP8 / 32768.0
	p = p + (v1+v2+c.digP7)/16.0
	pressure = p / 100.0
	return temperature, pressure, humidity
}
