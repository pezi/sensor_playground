// Compact MCP9808 driver, ported from the register access in the Python
// node (smbus2): a 2-byte read of the ambient temperature register 0x05,
// decoded as a 12-bit + sign two's-complement value in units of 1/16 °C
// (the alert flag bits 15..13 are masked off) — readings match the Python
// node.
package main

import (
	"sensorplayground/common"
)

const mcpRegAmbientTemp = 0x05

// decodeMCP9808Temp converts the raw 16-bit ambient temperature register
// word to °C: lower 12 bits are the magnitude in 1/16 °C, bit 12 is the
// sign (two's complement), bits 15..13 are alert flags and ignored.
func decodeMCP9808Temp(word uint16) float64 {
	temperature := float64(word&0x0FFF) / 16.0
	if word&0x1000 != 0 {
		temperature -= 256.0
	}
	return temperature
}

type MCP9808 struct {
	dev *common.I2CDevice
}

// NewMCP9808 opens the sensor on /dev/i2c-<bus> at 0x18.
func NewMCP9808(bus int) (*MCP9808, error) {
	dev, err := common.OpenI2C(bus, 0x18)
	if err != nil {
		return nil, err
	}
	return &MCP9808{dev: dev}, nil
}

func (s *MCP9808) Close() error { return s.dev.Close() }

// Read returns one ambient temperature measurement in °C.
func (s *MCP9808) Read() (float64, error) {
	raw, err := s.dev.ReadRegs(mcpRegAmbientTemp, 2)
	if err != nil {
		return 0, err
	}
	return decodeMCP9808Temp(uint16(raw[0])<<8 | uint16(raw[1])), nil
}
