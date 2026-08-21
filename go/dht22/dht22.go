// Driver for the DHT22 single-wire protocol via GPIO character-device
// edge events (see ../common).
package main

import (
	"fmt"
	"sync"
	"time"

	"sensorplayground/common"
)

const (
	// The DHT22 needs at least a 1 ms low start signal. The DHT11-compatible
	// 18 ms signal is also accepted and leaves generous scheduler margin.
	startSignal    = 18 * time.Millisecond
	readTimeout    = 25 * time.Millisecond
	fallingEdges   = 42
	bitThresholdNs = 100_000
	minInterval    = 2 * time.Second
	maxAge         = 30 * time.Second
)

// decodeFrame turns one kernel-timestamped falling-edge train into the
// sensor's five payload bytes and verifies its checksum.
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

// convertDHT22 decodes 16-bit tenths. Temperature uses a sign bit plus
// magnitude rather than two's complement.
func convertDHT22(buf [5]byte) (temperature, humidity float64) {
	humidity = float64(uint16(buf[0])<<8|uint16(buf[1])) / 10
	temperature = float64(uint16(buf[2]&0x7F)<<8|uint16(buf[3])) / 10
	if buf[2]&0x80 != 0 {
		temperature = -temperature
	}
	return temperature, humidity
}

// dht22Sensor implements the DHT22's two-second sampling limit and keeps
// the last successful reading for up to 30 seconds.
type dht22Sensor struct {
	chipPath string
	pin      int

	mu          sync.Mutex
	cached      map[string]any
	cachedAt    time.Time
	attemptedAt time.Time
}

// newDht22Sensor probes the line once so a wrong chip/pin fails at startup.
func newDht22Sensor(chipPath string, pin int) (*dht22Sensor, error) {
	line, err := common.OpenEventLine(chipPath, pin, false, false, false)
	if err != nil {
		return nil, err
	}
	line.Close()
	return &dht22Sensor{chipPath: chipPath, pin: pin}, nil
}

func (s *dht22Sensor) sample() ([5]byte, error) {
	var buf [5]byte
	out, err := common.OpenOutputLine(s.chipPath, s.pin, false)
	if err != nil {
		return buf, err
	}
	time.Sleep(startSignal)

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
			break
		}
		event, err := line.ReadEvent(remaining)
		if err != nil {
			return buf, err
		}
		if event == nil {
			break
		}
		if !event.Rising {
			falling = append(falling, event.TimestampNs)
		}
	}
	return decodeFrame(falling)
}

// read returns the full-key REST reading, or nil when the cache has gone
// stale without a successful sensor response.
func (s *dht22Sensor) read() map[string]any {
	s.mu.Lock()
	defer s.mu.Unlock()

	now := time.Now()
	if now.Sub(s.attemptedAt) >= minInterval {
		s.attemptedAt = now
		if buf, err := s.sample(); err != nil {
			fmt.Println("DHT read failed (retrying):", err)
		} else {
			temperature, humidity := convertDHT22(buf)
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
