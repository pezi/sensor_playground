// Compact AHT10/AHT20 driver, ported from the Python node's smbus2 code.
//
// Protocol (identical on both chips): trigger a measurement with
// 0xAC 0x33 0x00, wait ~80 ms, then read 6 bytes — a status byte followed
// by 20-bit humidity and 20-bit temperature. Only the calibrate opcode
// differs (AHT20: 0xBE, AHT10: 0xE1), so initialization tries both.
//
// The chips speak raw command writes and plain reads (no register map),
// so this driver opens /dev/i2c-N itself instead of using the
// register-oriented common.I2CDevice wrapper.
package main

import (
	"fmt"
	"os"
	"sync"
	"time"

	"golang.org/x/sys/unix"
)

const (
	ahtAddress    = 0x38   // fixed on both chips
	ahtStatusBusy = 0x80   // measurement not finished yet
	i2cSlave      = 0x0703 // linux/i2c-dev.h I2C_SLAVE
)

var (
	ahtCmdCalibrateAHT20 = []byte{0xBE, 0x08, 0x00}
	ahtCmdCalibrateAHT10 = []byte{0xE1, 0x08, 0x00}
	ahtCmdMeasure        = []byte{0xAC, 0x33, 0x00}
)

// AHT20Readings is one triggered measurement.
type AHT20Readings struct {
	Temperature float64 // °C
	Humidity    float64 // %RH
}

type AHT20 struct {
	f  ahtIO
	mu sync.Mutex
}

type ahtIO interface {
	Read([]byte) (int, error)
	Write([]byte) (int, error)
	Close() error
}

// NewAHT20 opens the sensor on /dev/i2c-<bus> at 0x38 and calibrates it.
func NewAHT20(bus int) (*AHT20, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, ahtAddress); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", ahtAddress, err)
	}
	s := &AHT20{f: f}
	time.Sleep(40 * time.Millisecond)
	s.calibrate()
	return s, nil
}

func (s *AHT20) Close() error { return s.f.Close() }

// calibrate sends the calibrate command; tolerate either chip variant.
func (s *AHT20) calibrate() {
	for _, command := range [][]byte{ahtCmdCalibrateAHT20, ahtCmdCalibrateAHT10} {
		if _, err := s.f.Write(command); err == nil {
			time.Sleep(10 * time.Millisecond)
			return
		}
	}
}

// Read triggers a measurement, waits ~80 ms and returns the readings, or
// (nil, nil) when the sensor is still busy (like the Python node's None).
func (s *AHT20) Read() (*AHT20Readings, error) {
	// UDP discovery and HTTPS requests can arrive concurrently. Keep the
	// trigger, conversion delay and response read as one sensor transaction.
	s.mu.Lock()
	defer s.mu.Unlock()

	if _, err := s.f.Write(ahtCmdMeasure); err != nil {
		return nil, err
	}
	time.Sleep(80 * time.Millisecond)
	raw := make([]byte, 6)
	if _, err := s.f.Read(raw); err != nil {
		return nil, err
	}
	temperature, humidity, ok := parseAHT20(raw)
	if !ok {
		return nil, nil
	}
	return &AHT20Readings{Temperature: temperature, Humidity: humidity}, nil
}

// parseAHT20 converts a raw 6-byte measurement (status, 20-bit humidity,
// 20-bit temperature) to (°C, %RH); ok is false while the sensor is busy.
func parseAHT20(raw []byte) (temperature, humidity float64, ok bool) {
	if raw[0]&ahtStatusBusy != 0 {
		return 0, 0, false
	}
	humRaw := int64(raw[1])<<12 | int64(raw[2])<<4 | int64(raw[3])>>4
	tempRaw := int64(raw[3]&0x0F)<<16 | int64(raw[4])<<8 | int64(raw[5])
	temperature = float64(tempRaw)/1048576.0*200.0 - 50.0
	humidity = float64(humRaw) / 1048576.0 * 100.0
	return temperature, humidity, true
}
