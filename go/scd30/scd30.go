// Compact SCD30 driver, ported from the scd30_i2c Python driver
// (https://github.com/RequestForCoffee/scd30, MIT) — the library the
// Python node uses: continuous measurement started at a 2 s interval,
// data-ready polling, Sensirion CRC-8 (poly 0x31, init 0xFF) on every
// 16-bit word, and the measurement floats assembled from two big-endian
// words (MSW first) into an IEEE-754 single.
//
// The SCD30 has no register map — commands are bare 16-bit big-endian
// writes (arguments carry their own CRC) and results are bare reads —
// so the driver talks to /dev/i2c-N directly instead of going through
// the common package's register-based I2C helper.
package main

import (
	"fmt"
	"math"
	"os"
	"time"

	"golang.org/x/sys/unix"
)

const (
	scd30Addr = 0x61

	scd30CmdStartPeriodic = 0x0010 // arg: ambient pressure mbar, 0 = disabled
	scd30CmdSetInterval   = 0x4600 // arg: measurement interval in seconds
	scd30CmdDataReady     = 0x0202
	scd30CmdReadMeasure   = 0x0300 // 6 words: co2, temperature, humidity

	scd30Interval = 2 // seconds, like the Python node

	i2cSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE
)

// scd30CRC8 is the Sensirion CRC-8: polynomial 0x31, init 0xFF (datasheet
// example: 0xBE 0xEF -> 0x92).
func scd30CRC8(data []byte) uint8 {
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

// parseSCD30Words validates the per-word CRCs of a response frame
// ([msb, lsb, crc] per word) and returns the big-endian 16-bit words.
func parseSCD30Words(frame []byte) ([]uint16, error) {
	if len(frame) == 0 || len(frame)%3 != 0 {
		return nil, fmt.Errorf("SCD30 frame has %d bytes, want a multiple of 3", len(frame))
	}
	words := make([]uint16, 0, len(frame)/3)
	for i := 0; i < len(frame); i += 3 {
		if scd30CRC8(frame[i:i+2]) != frame[i+2] {
			return nil, fmt.Errorf("SCD30 CRC mismatch in word %d", i/3)
		}
		words = append(words, uint16(frame[i])<<8|uint16(frame[i+1]))
	}
	return words, nil
}

// decodeSCD30Float assembles two big-endian 16-bit words (MSW first) into
// an IEEE-754 single-precision float, as the scd30_i2c driver does.
func decodeSCD30Float(msw, lsw uint16) float64 {
	return float64(math.Float32frombits(uint32(msw)<<16 | uint32(lsw)))
}

// decodeSCD30Measurement decodes the 18-byte read-measurement frame into
// (co2 ppm, temperature °C, humidity %RH).
func decodeSCD30Measurement(frame []byte) (co2, temperature, humidity float64, err error) {
	words, err := parseSCD30Words(frame)
	if err != nil {
		return 0, 0, 0, err
	}
	if len(words) != 6 {
		return 0, 0, 0, fmt.Errorf("SCD30 measurement has %d words, want 6", len(words))
	}
	co2 = decodeSCD30Float(words[0], words[1])
	temperature = decodeSCD30Float(words[2], words[3])
	humidity = decodeSCD30Float(words[4], words[5])
	return co2, temperature, humidity, nil
}

// SCD30 reads temperature, humidity, and CO2 from an SCD30.
type SCD30 struct {
	f *os.File
}

// NewSCD30 opens the sensor on /dev/i2c-<bus> at 0x61, sets the 2 s
// measurement interval and starts continuous measurement (ambient
// pressure compensation disabled), like the Python node.
func NewSCD30(bus int) (*SCD30, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, scd30Addr); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", scd30Addr, err)
	}
	s := &SCD30{f: f}
	// Set the interval, then read back and discard the echoed word (the
	// scd30_i2c driver does the same).
	if err := s.sendCommand(scd30CmdSetInterval, scd30Interval); err != nil {
		f.Close()
		return nil, err
	}
	if _, err := s.readWords(1); err != nil {
		f.Close()
		return nil, fmt.Errorf("setting SCD30 measurement interval: %w", err)
	}
	if err := s.sendCommand(scd30CmdStartPeriodic, 0); err != nil {
		f.Close()
		return nil, err
	}
	return s, nil
}

func (s *SCD30) Close() error { return s.f.Close() }

// sendCommand writes a big-endian command word plus CRC-protected
// arguments, then waits the >3 ms the datasheet requires between I2C
// transactions (the scd30_i2c driver waits 5 ms).
func (s *SCD30) sendCommand(cmd uint16, args ...uint16) error {
	msg := []byte{byte(cmd >> 8), byte(cmd)}
	for _, a := range args {
		word := []byte{byte(a >> 8), byte(a)}
		msg = append(msg, word[0], word[1], scd30CRC8(word))
	}
	if _, err := s.f.Write(msg); err != nil {
		return err
	}
	time.Sleep(5 * time.Millisecond)
	return nil
}

// readWords reads n CRC-protected words from the sensor.
func (s *SCD30) readWords(n int) ([]uint16, error) {
	frame := make([]byte, 3*n)
	if _, err := s.f.Read(frame); err != nil {
		return nil, err
	}
	return parseSCD30Words(frame)
}

// DataReady polls the data-ready status word.
func (s *SCD30) DataReady() (bool, error) {
	if err := s.sendCommand(scd30CmdDataReady); err != nil {
		return false, err
	}
	words, err := s.readWords(1)
	if err != nil {
		return false, err
	}
	return words[0] == 1, nil
}

// Read returns (co2 ppm, temperature °C, humidity %RH) once a fresh
// measurement is ready, or ok=false while none is (like the Python
// node's read() returning None).
func (s *SCD30) Read() (co2, temperature, humidity float64, ok bool, err error) {
	ready, err := s.DataReady()
	if err != nil {
		return 0, 0, 0, false, err
	}
	if !ready {
		return 0, 0, 0, false, nil
	}
	if err := s.sendCommand(scd30CmdReadMeasure); err != nil {
		return 0, 0, 0, false, err
	}
	frame := make([]byte, 18)
	if _, err := s.f.Read(frame); err != nil {
		return 0, 0, 0, false, err
	}
	co2, temperature, humidity, err = decodeSCD30Measurement(frame)
	if err != nil {
		return 0, 0, 0, false, err
	}
	return co2, temperature, humidity, true, nil
}
