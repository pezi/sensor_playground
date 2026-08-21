// Driver for the DHT11/DHT22 single-wire protocol via GPIO
// character-device edge events (see ../common).
//
// One read: drive the line low for ~18 ms (the start signal), release it,
// and capture the falling-edge train the sensor answers with. Each bit
// slot is a 50 µs low phase plus a 26-28 µs (0) or 70 µs (1) high phase,
// so the interval between consecutive falling edges is ~76-78 µs for a 0
// and ~120 µs for a 1 — told apart cleanly with the kernel's edge
// timestamps, where a userspace polling loop would drown the difference
// in jitter. The 40 bits carry five bytes (humidity, temperature, and a
// checksum) whose decoding differs between DHT11 and DHT22.
package main

import (
	"fmt"
	"strings"
	"sync"
	"time"

	"sensorplayground/common"
)

const (
	// Start signal: hold the line low for >=18 ms (DHT11; the DHT22
	// needs less). time.Sleep's overshoot is harmless — the signal only
	// has a minimum width.
	startSignal = 18 * time.Millisecond

	// The response preamble (~240 µs) plus 40 bits (<= ~4.8 ms) are long
	// over before this; a quiet line simply times out.
	readTimeout = 25 * time.Millisecond

	// Falling edges of one frame: the response preamble, 40 bit slots,
	// and the sensor's final low pull before releasing the bus.
	fallingEdges = 42

	// Between a 0 bit's ~76-78 µs and a 1 bit's ~120 µs interval.
	bitThresholdNs = 100_000
)

// decodeFrame turns the falling-edge timestamps of one read into the
// sensor's five payload bytes, MSB first. Only the last 41 edges are
// used (40 falling-to-falling intervals), so a missed or extra edge at
// the start — the response preamble, or noise from the host releasing
// the line — does not shift the bits.
func decodeFrame(fallingNs []uint64) ([5]byte, error) {
	var buf [5]byte
	if len(fallingNs) < 41 {
		return buf, fmt.Errorf("incomplete bit train (%d of %d falling edges)", len(fallingNs), fallingEdges)
	}
	bits := fallingNs[len(fallingNs)-41:]
	for i := 0; i < 40; i++ {
		if bits[i+1]-bits[i] >= bitThresholdNs {
			buf[i/8] |= 1 << (7 - i%8)
		}
	}
	if (int(buf[0])+int(buf[1])+int(buf[2])+int(buf[3]))&0xFF != int(buf[4]) {
		return buf, fmt.Errorf("checksum mismatch (% x)", buf)
	}
	return buf, nil
}

// convert decodes the five payload bytes into temperature [°C] and
// humidity [%RH]. The DHT11 sends whole values in bytes 0 and 2; the
// DHT22 sends 16-bit tenths, temperature as sign bit + magnitude — the
// same decoding adafruit-circuitpython-dht applies for the Python node.
func convert(buf [5]byte, dht22 bool) (temperature, humidity float64) {
	if !dht22 {
		return float64(buf[2]), float64(buf[0])
	}
	humidity = float64(uint16(buf[0])<<8|uint16(buf[1])) / 10
	temperature = float64(uint16(buf[2]&0x7F)<<8|uint16(buf[3])) / 10
	if buf[2]&0x80 != 0 {
		temperature = -temperature
	}
	return temperature, humidity
}

// dhtSensor reads a DHT11/DHT22 with the Python node's throttle-and-cache
// policy: the chip samples at most once per second (DHT11) / once per two
// seconds (DHT22), so polls are throttled to one hardware read every two
// seconds. A failed read keeps the previous values; the cache goes stale
// — and read() reports a failure — only after maxAge without a
// successful read.
type dhtSensor struct {
	chipPath string
	pin      int
	dht22    bool

	mu          sync.Mutex
	cached      map[string]any
	cachedAt    time.Time
	attemptedAt time.Time
}

const (
	minInterval = 2 * time.Second
	maxAge      = 30 * time.Second
)

// newDhtSensor probes the line once so a wrong chip/pin (or a non-Linux
// host) fails at startup instead of on every read.
func newDhtSensor(chipPath string, pin int, name string) (*dhtSensor, error) {
	line, err := common.OpenEventLine(chipPath, pin, false, false, false)
	if err != nil {
		return nil, err
	}
	line.Close()
	return &dhtSensor{chipPath: chipPath, pin: pin, dht22: strings.EqualFold(name, "DHT22")}, nil
}

// sample performs one single-wire read: start signal out, then decode the
// falling-edge train the sensor answers with.
func (s *dhtSensor) sample() ([5]byte, error) {
	var buf [5]byte

	// Start signal: requesting the line as output drives it low.
	out, err := common.OpenOutputLine(s.chipPath, s.pin, false)
	if err != nil {
		return buf, err
	}
	time.Sleep(startSignal)

	// Release the line (the module's pull-up raises it) and hand the pin
	// to the kernel's edge machinery. The sensor answers within ~40 µs
	// and the first data edge falls ~200 µs after the release, so the
	// re-request must win that race — when it loses, the frame comes up
	// short, the checksum rejects it, and the cache covers the retry.
	out.Close()
	line, err := common.OpenEventLine(s.chipPath, s.pin, false, false, false)
	if err != nil {
		return buf, err
	}
	defer line.Close()

	falling := make([]uint64, 0, fallingEdges)
	deadline := time.Now().Add(readTimeout)
	for len(falling) < fallingEdges {
		remaining := time.Until(deadline)
		if remaining <= 0 {
			break // decode what arrived
		}
		event, err := line.ReadEvent(remaining)
		if err != nil {
			return buf, err
		}
		if event == nil {
			break // timeout: decode what arrived
		}
		if !event.Rising {
			falling = append(falling, event.TimestampNs)
		}
	}
	return decodeFrame(falling)
}

// read returns the full-key readings for the REST API, or nil when the
// sensor has not answered for a while.
func (s *dhtSensor) read() map[string]any {
	s.mu.Lock()
	defer s.mu.Unlock()

	now := time.Now()
	if now.Sub(s.attemptedAt) >= minInterval {
		s.attemptedAt = now
		if buf, err := s.sample(); err != nil {
			// Single-wire reads fail now and then; the cache covers it.
			fmt.Println("DHT read failed (retrying):", err)
		} else {
			temperature, humidity := convert(buf, s.dht22)
			s.cached = map[string]any{
				"temperature": round1(temperature),
				"humidity":    round1(humidity),
			}
			s.cachedAt = now
		}
	}

	if s.cached == nil || now.Sub(s.cachedAt) > maxAge {
		return nil
	}
	return s.cached
}
