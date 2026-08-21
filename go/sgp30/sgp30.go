// Compact SGP30 driver, ported from the Pimoroni sgp30-python driver
// (https://github.com/pimoroni/sgp30-python, MIT) — the library the Python
// node uses — so the behavior matches it: big-endian 16-bit command words,
// CRC-8-checked response words, and the blocking warm-up that discards the
// sensor's initialization readings (fixed 400 ppm / 0 ppb).
//
// The SGP30 is not a register-map device — a command is a plain 2-byte
// write and the response a plain read — so this file opens /dev/i2c-N
// directly (same syscalls as the common package).
package main

import (
	"fmt"
	"os"
	"time"

	"golang.org/x/sys/unix"
)

const (
	sgp30Addr     = 0x58
	sgp30I2CSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE

	sgp30CmdInitAirQuality    = 0x2003 // no response words
	sgp30CmdMeasureAirQuality = 0x2008 // 2 response words: eCO2 ppm, TVOC ppb
)

// sgp30CRC calculates the 8-bit CRC of a 16-bit word as defined in section
// 6.6 of the SGP30 datasheet: polynomial 0x31 (x8 + x5 + x4 + 1),
// initialization 0xFF, no reflection, no final XOR.
func sgp30CRC(word uint16) uint8 {
	crc := uint8(0xFF)
	for _, b := range []uint8{uint8(word >> 8), uint8(word)} {
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

// parseSGP30Words verifies the per-word CRC of a response buffer (each
// 16-bit word is followed by its CRC byte) and returns the words.
func parseSGP30Words(buf []byte) ([]uint16, error) {
	words := make([]uint16, 0, len(buf)/3)
	for i := 0; i+2 < len(buf); i += 3 {
		word := uint16(buf[i])<<8 | uint16(buf[i+1])
		if buf[i+2] != sgp30CRC(word) {
			return nil, fmt.Errorf("invalid CRC in response from SGP30: %02x != %02x",
				buf[i+2], sgp30CRC(word))
		}
		words = append(words, word)
	}
	return words, nil
}

type SGP30 struct {
	f *os.File
}

// NewSGP30 opens the sensor on /dev/i2c-<bus> at 0x58.
func NewSGP30(bus int) (*SGP30, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), sgp30I2CSlave, sgp30Addr); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", sgp30Addr, err)
	}
	return &SGP30{f: f}, nil
}

func (s *SGP30) Close() error { return s.f.Close() }

// command writes a 16-bit command word, waits the reference driver's fixed
// 25 ms and reads responseWords CRC-checked 16-bit words back.
func (s *SGP30) command(cmd uint16, responseWords int) ([]uint16, error) {
	if _, err := s.f.Write([]byte{byte(cmd >> 8), byte(cmd)}); err != nil {
		return nil, err
	}
	time.Sleep(25 * time.Millisecond)
	if responseWords == 0 {
		return nil, nil
	}
	buf := make([]byte, responseWords*3)
	if _, err := s.f.Read(buf); err != nil {
		return nil, err
	}
	return parseSGP30Words(buf)
}

// StartMeasurement starts air quality measurement, mirroring the reference
// driver's start_measurement: after init_air_quality the SGP30 returns
// fixed 400 ppm / 0 ppb readings for about 15 s (page 8/15 of the
// datasheet), so readings are discarded until they change — capped at 20
// test samples to avoid a potential infinite loop.
func (s *SGP30) StartMeasurement() error {
	if _, err := s.command(sgp30CmdInitAirQuality, 0); err != nil {
		return err
	}
	for testSamples := 0; ; testSamples++ {
		words, err := s.command(sgp30CmdMeasureAirQuality, 2)
		if err != nil {
			return err
		}
		if words[0] != 400 || words[1] != 0 || testSamples >= 20 {
			return nil
		}
		time.Sleep(time.Second)
	}
}

// Read returns one air-quality measurement: eCO2 (ppm) and TVOC (ppb).
func (s *SGP30) Read() (eco2, tvoc uint16, err error) {
	words, err := s.command(sgp30CmdMeasureAirQuality, 2)
	if err != nil {
		return 0, 0, err
	}
	return words[0], words[1], nil
}
