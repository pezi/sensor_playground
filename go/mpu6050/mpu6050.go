// MPU6050 driver — the InvenSense 6-axis IMU, driven directly over I2C
// like the Python node's smbus2 access: the chip wakes from sleep by
// clearing PWR_MGMT_1, then a 14-byte burst read from ACCEL_XOUT_H
// delivers the accelerometer (big-endian 16-bit words, 16384 LSB/g at the
// default ±2 g range), temperature (raw/340 + 36.53 °C) and gyroscope
// values (the gyroscope words are part of the burst but unused, like the
// Python node).
package main

import (
	"math"

	"sensorplayground/common"
)

const (
	mpuRegPwrMgmt1   = 0x6B
	mpuRegAccelXOutH = 0x3B

	mpuAddress = 0x68
	mpuLSBPerG = 16384.0
)

// s16be decodes the big-endian two's-complement word at raw[i], raw[i+1].
func s16be(raw []byte, i int) int {
	value := int(raw[i])<<8 | int(raw[i+1])
	if value > 32767 {
		return value - 65536
	}
	return value
}

// decodeAccelTemp converts the 14-byte ACCEL_XOUT_H burst to axis g
// values plus the die temperature in °C.
func decodeAccelTemp(raw []byte) (x, y, z, temperature float64) {
	x = float64(s16be(raw, 0)) / mpuLSBPerG
	y = float64(s16be(raw, 2)) / mpuLSBPerG
	z = float64(s16be(raw, 4)) / mpuLSBPerG
	temperature = float64(s16be(raw, 6))/340.0 + 36.53
	return x, y, z, temperature
}

// orientation converts axis g values into roll / pitch angles (degrees)
// plus the total acceleration magnitude.
func orientation(x, y, z float64) (roll, pitch, gforce float64) {
	roll = math.Atan2(y, z) * 180 / math.Pi
	pitch = math.Atan2(-x, math.Sqrt(y*y+z*z)) * 180 / math.Pi
	gforce = math.Sqrt(x*x + y*y + z*z)
	return roll, pitch, gforce
}

// MPU6050Readings is one decoded orientation sample.
type MPU6050Readings struct {
	Temperature float64 // °C (chip die)
	Roll        float64 // degrees
	Pitch       float64 // degrees
	GForce      float64 // g
}

type MPU6050 struct {
	dev *common.I2CDevice
}

// NewMPU6050 opens the sensor on /dev/i2c-<bus> at 0x68 and wakes it (the
// chip powers up in sleep mode).
func NewMPU6050(bus int) (*MPU6050, error) {
	dev, err := common.OpenI2C(bus, mpuAddress)
	if err != nil {
		return nil, err
	}
	if err := dev.WriteReg(mpuRegPwrMgmt1, 0x00); err != nil {
		dev.Close()
		return nil, err
	}
	return &MPU6050{dev: dev}, nil
}

func (s *MPU6050) Close() error { return s.dev.Close() }

// Read returns die temperature and roll/pitch/g-force from one burst read.
func (s *MPU6050) Read() (*MPU6050Readings, error) {
	raw, err := s.dev.ReadRegs(mpuRegAccelXOutH, 14)
	if err != nil {
		return nil, err
	}
	x, y, z, temperature := decodeAccelTemp(raw)
	roll, pitch, gforce := orientation(x, y, z)
	return &MPU6050Readings{Temperature: temperature, Roll: roll, Pitch: pitch, GForce: gforce}, nil
}
