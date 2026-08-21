// BMP085 / BMP180 barometer driver — a direct port of the integer
// compensation algorithm written out in python/bmp085/sensor_node.py (from
// the BMP085 datasheet, section 3.5), so both nodes stay in step. The
// pin-compatible BMP180 uses the same registers and works unchanged.
package main

import (
	"fmt"
	"math"
	"time"

	"sensorplayground/common"
)

const (
	bmpI2CAddress = 0x77 // fixed — the BMP085 has no address pin

	bmpRegCalibration = 0xAA // 22 bytes: AC1..AC6, B1, B2, MB, MC, MD
	bmpRegChipID      = 0xD0 // reads 0x55 on a BMP085 (and on a BMP180)
	bmpRegControl     = 0xF4
	bmpRegData        = 0xF6

	bmpCmdReadTemperature = 0x2E
	bmpCmdReadPressure    = 0x34

	bmpChipID = 0x55

	// Standard sea-level pressure, in pascal. Altitude is relative to this,
	// so it moves with the weather as much as with the height.
	seaLevelPa = 101325.0
)

// Conversion time per oversampling setting (datasheet table 3, rounded up).
var bmpConversionTime = [4]time.Duration{
	5 * time.Millisecond, 8 * time.Millisecond, 14 * time.Millisecond, 26 * time.Millisecond,
}

// bmpCalibration holds the eleven factory constants from the sensor EEPROM.
type bmpCalibration struct {
	ac1, ac2, ac3, ac4, ac5, ac6 int64
	b1, b2, mb, mc, md           int64
}

// parseBMPCalibration parses the 22 calibration bytes. AC4, AC5 and AC6 are
// the only unsigned words. The datasheet states no calibration word is ever
// 0x0000 or 0xFFFF — exactly what a bus with nothing on it reads back — so
// that is rejected here rather than compensated with.
func parseBMPCalibration(data []byte) (bmpCalibration, error) {
	if len(data) != 22 {
		return bmpCalibration{}, fmt.Errorf("need 22 calibration bytes, got %d", len(data))
	}
	signedness := [11]bool{true, true, true, false, false, false, true, true, true, true, true}
	var values [11]int64
	for i, signed := range signedness {
		raw := uint16(data[i*2])<<8 | uint16(data[i*2+1])
		if raw == 0x0000 || raw == 0xFFFF {
			return bmpCalibration{}, fmt.Errorf(
				"implausible calibration word %d = 0x%04X (bad I2C read?)", i, raw)
		}
		if signed {
			values[i] = int64(int16(raw))
		} else {
			values[i] = int64(raw)
		}
	}
	return bmpCalibration{
		ac1: values[0], ac2: values[1], ac3: values[2], ac4: values[3],
		ac5: values[4], ac6: values[5], b1: values[6], b2: values[7],
		mb: values[8], mc: values[9], md: values[10],
	}, nil
}

// truncDiv divides truncating toward zero, as C's / does. Go's / already
// truncates, so this exists to mirror the Python node's _trunc_div and keep
// the transcriptions aligned line for line.
func truncDiv(numerator, denominator int64) int64 { return numerator / denominator }

// compensateBMP085 turns raw readings into (temperature °C, pressure Pa) —
// a direct transcription of the integer algorithm in the BMP085 datasheet.
func compensateBMP085(c *bmpCalibration, rawTemperature, rawPressure int64, oversampling uint) (float64, int64) {
	// Temperature.
	x1 := ((rawTemperature - c.ac6) * c.ac5) >> 15
	x2 := truncDiv(c.mc*2048, x1+c.md)
	b5 := x1 + x2
	temperature := float64((b5+8)>>4) / 10.0 // datasheet yields 0.1 °C steps

	// Pressure.
	b6 := b5 - 4000
	x1 = (c.b2 * ((b6 * b6) >> 12)) >> 11
	x2 = (c.ac2 * b6) >> 11
	x3 := x1 + x2
	b3 := (((c.ac1*4 + x3) << oversampling) + 2) >> 2
	x1 = (c.ac3 * b6) >> 13
	x2 = (c.b1 * ((b6 * b6) >> 12)) >> 16
	x3 = ((x1 + x2) + 2) >> 2
	b4 := (c.ac4 * (x3 + 32768)) >> 15
	b7 := (rawPressure - b3) * (50000 >> oversampling)
	// B7 is unsigned 32-bit in the datasheet and can exceed 2^31, which is
	// why it is scaled before rather than after the division in that case.
	var pressure int64
	if b7 < 0x80000000 {
		pressure = (b7 * 2) / b4
	} else {
		pressure = (b7 / b4) * 2
	}
	x1 = (pressure >> 8) * (pressure >> 8)
	x1 = (x1 * 3038) >> 16
	x2 = (-7357 * pressure) >> 16
	pressure += (x1 + x2 + 3791) >> 4

	return temperature, pressure
}

// altitudeFor converts pressure to altitude in metres, per the
// international barometric formula the BMP085 datasheet quotes.
func altitudeFor(pressurePa float64) float64 {
	return 44330.0 * (1.0 - math.Pow(pressurePa/seaLevelPa, 1.0/5.255))
}

type BMP085 struct {
	dev          *common.I2CDevice
	calib        bmpCalibration
	oversampling uint
}

// NewBMP085 opens the sensor on /dev/i2c-<bus> at 0x77, verifies the chip
// id and loads the calibration EEPROM.
func NewBMP085(bus int, oversampling int) (*BMP085, error) {
	if oversampling < 0 || oversampling > 3 {
		return nil, fmt.Errorf("oversampling must be 0-3, got %d", oversampling)
	}
	dev, err := common.OpenI2C(bus, bmpI2CAddress)
	if err != nil {
		return nil, err
	}
	id, err := dev.ReadReg(bmpRegChipID)
	if err != nil {
		dev.Close()
		return nil, err
	}
	if id != bmpChipID {
		dev.Close()
		return nil, fmt.Errorf(
			"no BMP085 at address %#04x: chip id is %#04x, expected %#04x",
			bmpI2CAddress, id, bmpChipID)
	}
	raw, err := dev.ReadRegs(bmpRegCalibration, 22)
	if err != nil {
		dev.Close()
		return nil, err
	}
	calib, err := parseBMPCalibration(raw)
	if err != nil {
		dev.Close()
		return nil, err
	}
	return &BMP085{dev: dev, calib: calib, oversampling: uint(oversampling)}, nil
}

func (s *BMP085) Close() error { return s.dev.Close() }

func (s *BMP085) readRawTemperature() (int64, error) {
	if err := s.dev.WriteReg(bmpRegControl, bmpCmdReadTemperature); err != nil {
		return 0, err
	}
	time.Sleep(bmpConversionTime[0]) // temperature ignores oversampling
	data, err := s.dev.ReadRegs(bmpRegData, 2)
	if err != nil {
		return 0, err
	}
	return int64(data[0])<<8 | int64(data[1]), nil
}

func (s *BMP085) readRawPressure() (int64, error) {
	cmd := uint8(bmpCmdReadPressure + (s.oversampling << 6))
	if err := s.dev.WriteReg(bmpRegControl, cmd); err != nil {
		return 0, err
	}
	time.Sleep(bmpConversionTime[s.oversampling])
	data, err := s.dev.ReadRegs(bmpRegData, 3)
	if err != nil {
		return 0, err
	}
	raw := int64(data[0])<<16 | int64(data[1])<<8 | int64(data[2])
	return raw >> (8 - s.oversampling), nil
}

// Read returns (temperature °C, pressure Pa). Temperature is read first,
// and every time: its B5 term feeds the pressure compensation, so a stale
// one skews the pressure as the chip warms.
func (s *BMP085) Read() (float64, int64, error) {
	rawT, err := s.readRawTemperature()
	if err != nil {
		return 0, 0, err
	}
	rawP, err := s.readRawPressure()
	if err != nil {
		return 0, 0, err
	}
	temperature, pressure := compensateBMP085(&s.calib, rawT, rawP, s.oversampling)
	return temperature, pressure, nil
}
