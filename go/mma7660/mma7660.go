// MMA7660 driver — the Grove 3-Axis Digital Accelerometer ±1.5g
// (MMA7660FC), driven directly over I2C like the Python node's smbus2
// access: each axis is a 6-bit two's-complement value with 21.33 counts
// per g; bit 6 of a sample is the alert flag, meaning the register was
// updated mid-read and must be read again.
package main

import (
	"fmt"
	"math"

	"sensorplayground/common"
)

const (
	mmaRegX    = 0x00
	mmaRegMode = 0x07
	mmaRegSR   = 0x08

	mmaAddress    = 0x4C
	mmaCountsPerG = 21.33
	mmaAlertBit   = 0x40
)

// decodeAxis converts one 6-bit two's-complement sample to counts.
func decodeAxis(raw uint8) int {
	if raw > 31 {
		return int(raw) - 64
	}
	return int(raw)
}

// decodeAxes converts a 3-byte X/Y/Z block to g values; ok is false when
// any sample carries the alert bit (updated mid-read — read again).
func decodeAxes(raw []byte) (x, y, z float64, ok bool) {
	for _, v := range raw {
		if v&mmaAlertBit != 0 {
			return 0, 0, 0, false
		}
	}
	g := func(v uint8) float64 { return float64(decodeAxis(v)) / mmaCountsPerG }
	return g(raw[0]), g(raw[1]), g(raw[2]), true
}

// orientation converts axis g values into roll / pitch angles (degrees)
// plus the total acceleration magnitude.
func orientation(x, y, z float64) (roll, pitch, gforce float64) {
	roll = math.Atan2(y, z) * 180 / math.Pi
	pitch = math.Atan2(-x, math.Sqrt(y*y+z*z)) * 180 / math.Pi
	gforce = math.Sqrt(x*x + y*y + z*z)
	return roll, pitch, gforce
}

// MMA7660Readings is one decoded orientation sample.
type MMA7660Readings struct {
	Roll   float64 // degrees
	Pitch  float64 // degrees
	GForce float64 // g
}

type MMA7660 struct {
	dev *common.I2CDevice
}

// NewMMA7660 opens the sensor on /dev/i2c-<bus> at 0x4C and switches it
// active.
func NewMMA7660(bus int) (*MMA7660, error) {
	dev, err := common.OpenI2C(bus, mmaAddress)
	if err != nil {
		return nil, err
	}
	// Standby to configure, 32 samples/s, then active mode.
	for _, w := range [][2]uint8{{mmaRegMode, 0x00}, {mmaRegSR, 0x02}, {mmaRegMode, 0x01}} {
		if err := dev.WriteReg(w[0], w[1]); err != nil {
			dev.Close()
			return nil, fmt.Errorf("configuring MMA7660: %w", err)
		}
	}
	return &MMA7660{dev: dev}, nil
}

func (s *MMA7660) Close() error { return s.dev.Close() }

// Read returns roll/pitch/g-force, re-reading while the alert bit is set.
func (s *MMA7660) Read() (*MMA7660Readings, error) {
	for i := 0; i < 10; i++ {
		raw, err := s.dev.ReadRegs(mmaRegX, 3)
		if err != nil {
			return nil, err
		}
		x, y, z, ok := decodeAxes(raw)
		if !ok {
			continue
		}
		roll, pitch, gforce := orientation(x, y, z)
		return &MMA7660Readings{Roll: roll, Pitch: pitch, GForce: gforce}, nil
	}
	return nil, fmt.Errorf("MMA7660 kept reporting the alert bit")
}
