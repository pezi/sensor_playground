// Bit-banged driver for the Sensirion SHT1x two-wire protocol
// (datasheet V5), ported from the Python node. The bus is fully
// master-clocked with no minimum speed, so the pace of one GPIO
// character-device call per clock edge is harmless — the sensor simply
// waits between edges. The DATA line is driven open-drain style:
// released (input with pull-up) for a 1, actively pulled LOW for a 0,
// because the sensor drives the same wire when answering. Each
// direction flip re-requests the line on /dev/gpiochipN — the same mode
// flips the Python node does via lgpio claims.
//
// The conversion math is platform-neutral (and unit tested); the GPIO
// access needs Linux and fails cleanly elsewhere (via ../common).
package main

import (
	"fmt"
	"math"
	"sync"
	"time"

	"sensorplayground/common"
)

// -- SHT1x protocol constants (datasheet V5) ----------------------------------

const (
	cmdTemperature = 0x03 // 000 00011: measure temperature (14 bit)
	cmdHumidity    = 0x05 // 000 00101: measure humidity (12 bit)

	// A 14-bit measurement takes up to 320 ms.
	measureTimeout = 400 * time.Millisecond

	// Self-heating limit: at most one hardware read every two seconds,
	// serving the cache — which goes stale after maxAge without a
	// successful read.
	minInterval = 2 * time.Second
	maxAge      = 30 * time.Second
)

// 14-bit temperature at 3.3V supply: T = d1 + t1 * raw.
const d1 = -39.66

// 12-bit humidity polynomial + temperature compensation.
const (
	c1 = -2.0468
	c2 = 0.0367
	c3 = -1.5955e-6
	t1 = 0.01
	t2 = 0.00008
)

// sht1xTemperature converts a raw 14-bit temperature reading to °C.
func sht1xTemperature(raw uint16) float64 { return d1 + t1*float64(raw) }

// sht1xHumidity converts a raw 12-bit humidity reading to %RH (compensated).
func sht1xHumidity(raw uint16, temperature float64) float64 {
	r := float64(raw)
	linear := c1 + c2*r + c3*r*r
	compensated := (temperature-25.0)*(t1+t2*r) + linear
	return math.Max(0.0, math.Min(100.0, compensated))
}

// -- Sensor -------------------------------------------------------------------

// SHT1x bit-bangs the two-wire protocol on two GPIOs. Line helpers keep
// the first error of a transaction (sticky) so the protocol code reads
// like the waveform instead of drowning in error plumbing.
type SHT1x struct {
	chipPath string
	dataPin  int
	sckPin   int
	sck      *common.GpioLine
	data     *common.GpioLine // the current request on the DATA line
	err      error            // first error of the running transaction

	mu          sync.Mutex // discovery and REST poll concurrently
	cached      map[string]any
	cachedAt    time.Time
	attemptedAt time.Time
}

// NewSHT1x claims the two lines and resets the sensor interface. A wrong
// chip/pin (or a non-Linux host) fails here instead of on every read.
func NewSHT1x(chipPath string, dataPin, sckPin int) (*SHT1x, error) {
	sck, err := common.OpenOutputLine(chipPath, sckPin, false)
	if err != nil {
		return nil, err
	}
	s := &SHT1x{chipPath: chipPath, dataPin: dataPin, sckPin: sckPin, sck: sck}
	s.dataRelease()
	if s.err != nil {
		s.Close()
		return nil, s.err
	}
	// The sensor needs 11 ms after power-up before the first command.
	time.Sleep(20 * time.Millisecond)
	s.connectionReset()
	if s.err != nil {
		s.Close()
		return nil, s.err
	}
	return s, nil
}

// -- line helpers: DATA is bidirectional and never driven HIGH ----------------

// dataRelease releases DATA (the pull-up makes it HIGH, the sensor may
// drive it). OpenEventLine is the only common helper that requests a
// plain (non-inverting) input with the internal pull-up; its edge
// reporting is simply not used here.
func (s *SHT1x) dataRelease() {
	if s.err != nil {
		return
	}
	if s.data != nil {
		s.data.Close()
	}
	s.data, s.err = common.OpenEventLine(s.chipPath, s.dataPin, false, true, false)
}

// dataLow actively pulls DATA LOW (the line is requested as output; the
// kernel drives a fresh output request low).
func (s *SHT1x) dataLow() {
	if s.err != nil {
		return
	}
	if s.data != nil {
		s.data.Close()
	}
	s.data, s.err = common.OpenOutputLine(s.chipPath, s.dataPin, false)
}

// dataRead samples the DATA line.
func (s *SHT1x) dataRead() bool {
	if s.err != nil {
		return false
	}
	level, err := s.data.Get()
	if err != nil {
		s.err = err
		return false
	}
	return level
}

func (s *SHT1x) sckWrite(level bool) {
	if s.err != nil {
		return
	}
	s.err = s.sck.Set(level)
}

func (s *SHT1x) sckPulse() {
	s.sckWrite(true)
	s.sckWrite(false)
}

// -- protocol -----------------------------------------------------------------

// transmissionStart: DATA falls and rises while SCK is high — the start
// pattern.
func (s *SHT1x) transmissionStart() {
	s.dataRelease()
	s.sckWrite(false)
	s.sckWrite(true)
	s.dataLow()
	s.sckWrite(false)
	s.sckWrite(true)
	s.dataRelease()
	s.sckWrite(false)
}

// connectionReset resynchronises the interface: DATA high, nine or more
// clocks.
func (s *SHT1x) connectionReset() {
	s.dataRelease()
	s.sckWrite(false)
	for i := 0; i < 10; i++ {
		s.sckPulse()
	}
}

// sendCommand sends one command byte; returns false without the
// sensor's ACK.
func (s *SHT1x) sendCommand(command byte) bool {
	s.transmissionStart()
	for bit := 7; bit >= 0; bit-- {
		if command&(1<<bit) != 0 {
			s.dataRelease()
		} else {
			s.dataLow()
		}
		s.sckPulse()
	}
	// ACK: the sensor pulls DATA low during the ninth clock.
	s.dataRelease()
	s.sckWrite(true)
	acked := !s.dataRead()
	s.sckWrite(false)
	return acked && s.err == nil
}

// readByte reads one byte; [ack] keeps the transfer going, its absence
// ends it (the sensor then skips the CRC byte, which is not used here).
func (s *SHT1x) readByte(ack bool) byte {
	var value byte
	s.dataRelease()
	for bit := 7; bit >= 0; bit-- {
		s.sckWrite(true)
		if s.dataRead() {
			value |= 1 << bit
		}
		s.sckWrite(false)
	}
	if ack {
		s.dataLow()
	} else {
		s.dataRelease()
	}
	s.sckPulse()
	s.dataRelease()
	return value
}

// measure runs one measurement; returns the raw result or an error.
func (s *SHT1x) measure(command byte) (uint16, error) {
	s.err = nil
	if !s.sendCommand(command) {
		if s.err != nil {
			return 0, s.err
		}
		return 0, fmt.Errorf("no ACK for command 0x%02x", command)
	}
	// The sensor releases DATA while measuring, pulls it low when done.
	deadline := time.Now().Add(measureTimeout)
	for s.dataRead() {
		if s.err != nil {
			return 0, s.err
		}
		if time.Now().After(deadline) {
			return 0, fmt.Errorf("measurement timeout")
		}
		time.Sleep(5 * time.Millisecond)
	}
	msb := s.readByte(true)
	lsb := s.readByte(false)
	if s.err != nil {
		return 0, s.err
	}
	return uint16(msb)<<8 | uint16(lsb), nil
}

// -- public API ---------------------------------------------------------------

// Read returns the full-key readings for the REST API, or nil when the
// sensor has not answered for a while. Polls are throttled to one
// hardware read every two seconds (self-heating limit); a failed read
// keeps the previous values, and the cache goes stale — and Read
// reports a failure — only after maxAge without a successful read.
func (s *SHT1x) Read() map[string]any {
	s.mu.Lock()
	defer s.mu.Unlock()

	now := time.Now()
	if now.Sub(s.attemptedAt) >= minInterval {
		s.attemptedAt = now
		rawTemperature, err := s.measure(cmdTemperature)
		var rawHumidity uint16
		if err == nil {
			rawHumidity, err = s.measure(cmdHumidity)
		}
		if err != nil {
			fmt.Println("SHT1x read failed (serving cache):", err)
			s.err = nil
			s.connectionReset()
		} else {
			temperature := sht1xTemperature(rawTemperature)
			s.cached = map[string]any{
				"temperature": round1(temperature),
				"humidity":    round1(sht1xHumidity(rawHumidity, temperature)),
			}
			s.cachedAt = now
		}
	}

	if s.cached == nil || now.Sub(s.cachedAt) > maxAge {
		return nil
	}
	return s.cached
}

func (s *SHT1x) Close() {
	if s.data != nil {
		s.data.Close()
	}
	s.sck.Close()
}
