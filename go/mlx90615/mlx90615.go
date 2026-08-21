// Compact MLX90615 driver, ported from the register access in the Python
// node (smbus2): SMBus word reads from RAM register 0x26 (ambient) and
// 0x27 (object), temperature = raw * 0.02 K - 273.15, bit 15 set marks an
// error — readings match the Python node.
//
// The MLX90615 is a strict SMBus part: the register address and the data
// read must be one transfer with a repeated start, so this driver uses
// common.ReadWordSMBus rather than the plain write-then-read of ReadRegs.
//
// The conversion is platform-neutral (and unit tested); only the I2C
// access needs hardware.
package main

import (
	"sensorplayground/common"
)

const (
	mlxRegAmbient = 0x26
	mlxRegObject  = 0x27
)

// decodeMLX90615Temp converts a raw RAM word to °C, or reports notValid
// when the sensor flags an error (bit 15).
func decodeMLX90615Temp(raw uint16) (temperature float64, valid bool) {
	if raw&0x8000 != 0 {
		return 0, false
	}
	// The RAM value is the absolute temperature in units of 0.02 K.
	return float64(raw)*0.02 - 273.15, true
}

type MLX90615 struct {
	dev *common.I2CDevice
}

// NewMLX90615 opens the sensor on /dev/i2c-<bus> at 0x5B.
func NewMLX90615(bus int) (*MLX90615, error) {
	dev, err := common.OpenI2C(bus, 0x5B)
	if err != nil {
		return nil, err
	}
	return &MLX90615{dev: dev}, nil
}

func (s *MLX90615) Close() error { return s.dev.Close() }

// readTemperature returns one temperature in °C; valid is false when the
// sensor flagged an error.
func (s *MLX90615) readTemperature(register uint8) (temperature float64, valid bool, err error) {
	raw, err := s.dev.ReadWordSMBus(register)
	if err != nil {
		return 0, false, err
	}
	temperature, valid = decodeMLX90615Temp(raw)
	return temperature, valid, nil
}

// Read returns the sensor's own ambient temperature and the non-contact
// object temperature, both in °C.
func (s *MLX90615) Read() (ambient, object float64, valid bool, err error) {
	ambient, ambientValid, err := s.readTemperature(mlxRegAmbient)
	if err != nil {
		return 0, 0, false, err
	}
	object, objectValid, err := s.readTemperature(mlxRegObject)
	if err != nil {
		return 0, 0, false, err
	}
	return ambient, object, ambientValid && objectValid, nil
}
