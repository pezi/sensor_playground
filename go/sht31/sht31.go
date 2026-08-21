// Minimal SHT31 driver, ported from the Python node: single-shot
// measurement, high repeatability, no clock stretching. The port
// additionally verifies the Sensirion CRC-8 of both data words (the
// Python node reads but does not check the CRC bytes).
//
// The SHT31 is command-based, not register-mapped: reading a measurement
// is a plain 6-byte I2C read with no preceding register write, which the
// common I2CDevice wrapper cannot express — so this driver opens
// /dev/i2c-N itself (same I2C_SLAVE ioctl approach as common/i2c.go).
package main

import (
	"fmt"
	"os"
	"time"

	"golang.org/x/sys/unix"
)

const (
	sht31Addr = 0x44
	i2cSlave  = 0x0703 // linux/i2c-dev.h I2C_SLAVE
)

// sht31MeasureCmd is the single-shot measurement command: high
// repeatability, no clock stretching (like the Python node).
var sht31MeasureCmd = []byte{0x24, 0x00}

// sht31CRC computes the Sensirion CRC-8 (polynomial 0x31, init 0xFF) that
// protects each 16-bit word of the measurement frame.
func sht31CRC(data []byte) uint8 {
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

func sht31Temperature(raw uint16) float64 { return -45.0 + 175.0*float64(raw)/65535.0 }
func sht31Humidity(raw uint16) float64    { return 100.0 * float64(raw) / 65535.0 }

// parseSHT31Frame checks both CRCs of a 6-byte measurement frame
// (temp msb, temp lsb, crc, hum msb, hum lsb, crc) and converts the raw
// words with the datasheet formulas: (temperature °C, humidity %RH).
func parseSHT31Frame(frame []byte) (float64, float64, error) {
	if len(frame) != 6 {
		return 0, 0, fmt.Errorf("short SHT31 frame: %d bytes", len(frame))
	}
	if sht31CRC(frame[0:2]) != frame[2] {
		return 0, 0, fmt.Errorf("SHT31 temperature CRC mismatch")
	}
	if sht31CRC(frame[3:5]) != frame[5] {
		return 0, 0, fmt.Errorf("SHT31 humidity CRC mismatch")
	}
	tempRaw := uint16(frame[0])<<8 | uint16(frame[1])
	humRaw := uint16(frame[3])<<8 | uint16(frame[4])
	return sht31Temperature(tempRaw), sht31Humidity(humRaw), nil
}

type SHT31 struct {
	f *os.File
}

// NewSHT31 opens the sensor on /dev/i2c-<bus> at 0x44.
func NewSHT31(bus int) (*SHT31, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, sht31Addr); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", sht31Addr, err)
	}
	return &SHT31{f: f}, nil
}

func (s *SHT31) Close() error { return s.f.Close() }

// Read triggers a single-shot measurement, waits the conversion time and
// returns (temperature °C, humidity %RH).
func (s *SHT31) Read() (float64, float64, error) {
	if _, err := s.f.Write(sht31MeasureCmd); err != nil {
		return 0, 0, err
	}
	// The Python node's conversion wait (datasheet max 15 ms, rounded up).
	time.Sleep(20 * time.Millisecond)
	buf := make([]byte, 6)
	if _, err := s.f.Read(buf); err != nil {
		return 0, 0, err
	}
	return parseSHT31Frame(buf)
}
