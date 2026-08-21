// Compact Chirp driver, ported from the register access in the Python
// node (smbus2) — same register map as the Arduino reference library
// (https://github.com/Apollon77/I2CSoilMoistureSensor): a reset at
// startup, 16-bit big-endian reads of the capacitance, temperature and
// light registers, and the light measurement started by writing register
// 0x03. The capacitance mapping and the temperature decoding are
// platform-neutral (and unit tested); only the I2C access is Linux-only.
//
// The chip answers a register read as a separate transaction after a
// short pause (the Arduino library waits 20 ms), so this driver opens
// /dev/i2c-N itself instead of using the combined-transaction
// common.I2CDevice wrapper — and it needs the plain one-byte writes
// (SMBus "send byte") that the reset and light commands are.
package main

import (
	"fmt"
	"math"
	"os"
	"sync"
	"time"

	"golang.org/x/sys/unix"
)

const (
	regGetCapacitance = 0x00
	regMeasureLight   = 0x03
	regGetLight       = 0x04
	regGetTemperature = 0x05
	regReset          = 0x06
	regGetVersion     = 0x07

	// A light measurement takes up to three seconds on the chip.
	lightMeasureTime = 3 * time.Second

	// The chip needs a moment between the register write and the read;
	// the Arduino reference library waits 20 ms (the Python node instead
	// gets one combined SMBus transaction from smbus2).
	readDelay = 20 * time.Millisecond

	// The chip needs a moment after a reset.
	resetDelay = 1 * time.Second

	i2cSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE
)

// moisturePercent maps a raw capacitance onto 0-100 % between the dry and
// wet calibration points (the capacitance rises with moisture, so
// wet > dry); values outside the calibrated span clamp to 0/100 %.
func moisturePercent(capacitance, capDry, capWet int) int {
	span := float64(capWet - capDry)
	p := 100 * float64(capacitance-capDry) / span
	return int(math.Round(math.Min(100, math.Max(0, p))))
}

// decodeWord assembles the chip's big-endian 16-bit register value.
func decodeWord(hi, lo byte) uint16 { return uint16(hi)<<8 | uint16(lo) }

// decodeTemperature converts the raw temperature register word to °C:
// the chip reports a signed 16-bit value in tenths of a degree.
func decodeTemperature(word uint16) float64 {
	return round1(float64(int16(word)) / 10.0)
}

// lightCounts turns the raw light register value into brightness counts:
// the chip times a phototransistor discharge and so counts *up* in
// darkness, which the node inverts (higher = brighter).
func lightCounts(raw uint16) int { return 65535 - int(raw) }

// ChirpReading is one set of readings. Light is only valid once the
// first (up to three seconds long) light measurement has completed.
type ChirpReading struct {
	Capacitance int
	Temperature float64 // °C
	Light       int     // inverted brightness counts, higher = brighter
	HasLight    bool
}

type Chirp struct {
	mu           sync.Mutex
	f            *os.File
	light        int
	hasLight     bool
	lightStarted time.Time
	lightPending bool
}

// NewChirp opens the sensor on /dev/i2c-<bus>, resets it and reports the
// firmware version.
func NewChirp(bus, address int) (*Chirp, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, address); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", address, err)
	}
	s := &Chirp{f: f}
	if err := s.writeByte(regReset); err != nil {
		f.Close()
		return nil, err
	}
	time.Sleep(resetDelay)
	version, err := s.readU16(regGetVersion)
	if err != nil {
		f.Close()
		return nil, err
	}
	fmt.Printf("Chirp sensor at 0x%02x, firmware version 0x%02x\n", address, version&0xFF)
	return s, nil
}

func (s *Chirp) Close() error { return s.f.Close() }

// writeByte sends a bare command byte (SMBus "send byte").
func (s *Chirp) writeByte(value byte) error {
	_, err := s.f.Write([]byte{value})
	return err
}

// readU16 reads a big-endian 16-bit register.
func (s *Chirp) readU16(register byte) (uint16, error) {
	if _, err := s.f.Write([]byte{register}); err != nil {
		return 0, err
	}
	time.Sleep(readDelay)
	buf := make([]byte, 2)
	if _, err := s.f.Read(buf); err != nil {
		return 0, err
	}
	return decodeWord(buf[0], buf[1]), nil
}

// updateLight harvests a finished light measurement and starts the next
// one. The measurement runs on the chip, so this never blocks; the first
// call only starts one and leaves the value unset.
func (s *Chirp) updateLight() error {
	now := time.Now()
	if s.lightPending && now.Sub(s.lightStarted) >= lightMeasureTime {
		raw, err := s.readU16(regGetLight)
		if err != nil {
			return err
		}
		s.light = lightCounts(raw)
		s.hasLight = true
		s.lightPending = false
	}
	if !s.lightPending {
		if err := s.writeByte(regMeasureLight); err != nil {
			return err
		}
		s.lightStarted = now
		s.lightPending = true
	}
	return nil
}

// Read returns one set of readings; the REST and discovery paths share
// the sensor, so the light state machine is mutex-guarded.
func (s *Chirp) Read() (ChirpReading, error) {
	s.mu.Lock()
	defer s.mu.Unlock()

	if err := s.updateLight(); err != nil {
		return ChirpReading{}, err
	}
	capacitance, err := s.readU16(regGetCapacitance)
	if err != nil {
		return ChirpReading{}, err
	}
	raw, err := s.readU16(regGetTemperature)
	if err != nil {
		return ChirpReading{}, err
	}
	return ChirpReading{
		Capacitance: int(capacitance),
		Temperature: decodeTemperature(raw),
		Light:       s.light,
		HasLight:    s.hasLight,
	}, nil
}
