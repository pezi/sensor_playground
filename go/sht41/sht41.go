// Compact SHT41 driver (single-shot mode), ported from the Python node:
// high-precision measurement command 0xFD (~8 ms), then a 6-byte frame
// [temp msb, temp lsb, crc, hum msb, hum lsb, crc] converted with the
// datasheet formulas. Unlike the Python node this port also validates the
// two Sensirion CRC-8 checksums (poly 0x31, init 0xFF).
//
// The SHT41 has no register map — the measurement command is a bare
// single-byte write and the result a bare 6-byte read (which the sensor
// NACKs while still measuring), so the driver talks to /dev/i2c-N
// directly instead of going through the common package's register-based
// I2C helper.
package main

import (
	"fmt"
	"os"
	"time"

	"golang.org/x/sys/unix"
)

const (
	sht41Addr        = 0x44
	sht41MeasureHigh = 0xFD // high-precision single-shot measurement, ~8 ms

	i2cSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE
)

// sht41CRC8 is the Sensirion CRC-8: polynomial 0x31, init 0xFF (datasheet
// example: 0xBE 0xEF -> 0x92).
func sht41CRC8(data []byte) uint8 {
	crc := uint8(0xFF)
	for _, b := range data {
		crc ^= b
		for i := 0; i < 8; i++ {
			if crc&0x80 != 0 {
				crc = crc<<1 ^ 0x31
			} else {
				crc <<= 1
			}
		}
	}
	return crc
}

// parseSHT41Frame validates the CRCs of a 6-byte measurement frame and
// returns the raw temperature and humidity words.
func parseSHT41Frame(frame []byte) (tempRaw, humRaw uint16, err error) {
	if len(frame) != 6 {
		return 0, 0, fmt.Errorf("SHT41 frame has %d bytes, want 6", len(frame))
	}
	if sht41CRC8(frame[0:2]) != frame[2] || sht41CRC8(frame[3:5]) != frame[5] {
		return 0, 0, fmt.Errorf("SHT41 CRC mismatch")
	}
	tempRaw = uint16(frame[0])<<8 | uint16(frame[1])
	humRaw = uint16(frame[3])<<8 | uint16(frame[4])
	return tempRaw, humRaw, nil
}

// convertSHT41 converts the raw words with the datasheet formulas the
// Python node uses: temperature in °C, humidity in %RH clamped to 0..100.
func convertSHT41(tempRaw, humRaw uint16) (temperature, humidity float64) {
	temperature = -45.0 + 175.0*float64(tempRaw)/65535.0
	humidity = -6.0 + 125.0*float64(humRaw)/65535.0
	humidity = min(100.0, max(0.0, humidity))
	return temperature, humidity
}

// SHT41 reads temperature and humidity from an SHT41 (single-shot mode).
type SHT41 struct {
	f *os.File
}

// NewSHT41 opens the sensor on /dev/i2c-<bus> at 0x44.
func NewSHT41(bus int) (*SHT41, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, sht41Addr); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", sht41Addr, err)
	}
	return &SHT41{f: f}, nil
}

func (s *SHT41) Close() error { return s.f.Close() }

// Read triggers a high-precision single-shot measurement and returns
// (temperature °C, humidity %RH).
func (s *SHT41) Read() (temperature, humidity float64, err error) {
	if _, err := s.f.Write([]byte{sht41MeasureHigh}); err != nil {
		return 0, 0, err
	}
	time.Sleep(10 * time.Millisecond)
	frame := make([]byte, 6)
	if _, err := s.f.Read(frame); err != nil {
		return 0, 0, err
	}
	tempRaw, humRaw, err := parseSHT41Frame(frame)
	if err != nil {
		return 0, 0, err
	}
	temperature, humidity = convertSHT41(tempRaw, humRaw)
	return temperature, humidity, nil
}
