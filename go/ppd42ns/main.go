// Sensor Playground Sensor Node — Grove Dust Sensor / Shinyei PPD42NS (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a Grove Dust Sensor (Shinyei
// PPD42NS). The sensor pulls its output pin LOW while particles scatter
// light inside its chamber; the node accumulates that low-pulse occupancy
// (LPO) over 30-second windows and converts the ratio into a particle
// concentration in pcs/0.01cf using the Nafis curve, reported under the
// JSON key `dust`:
//
//	ratio         = low_time / window_time * 100          (percent)
//	concentration = 1.1*r^3 - 3.8*r^2 + 520*r + 0.62      (pcs/0.01cf)
//
// https://wiki.seeedstudio.com/Grove-Dust_Sensor/
// https://www.howmuchsnow.com/arduino/airquality/grovedust/
//
// The pin is read directly via kernel-timestamped GPIO edge events
// (character device, see ../common) — there is no extension-hat option:
// the Arduino-based hats are polled over I2C and cannot timestamp the
// sensor's 10-90 ms pulses.
//
// The first reading appears after the first full 30-second window; until
// then the REST endpoint answers 503 and the discovery reply carries no
// `dust` value. That is warm-up, not an error.
//
//   - HTTPS REST API on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     Wi-Fi with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to generate plausible readings
// without the sensor hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_ppd42ns
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"sync"
	"time"

	"golang.org/x/sys/unix"

	"sensorplayground/common"
)

func round1(x float64) float64 { return math.Round(x*10) / 10 }

func uniform(lo, hi float64) float64 { return lo + rand.Float64()*(hi-lo) }

// -- Sensor ------------------------------------------------------------------

const windowSeconds = 30.0

// concentration converts a low-pulse-occupancy ratio [%] into a particle
// concentration [pcs/0.01cf] — the Nafis curve.
func concentration(ratio float64) float64 {
	return 1.1*ratio*ratio*ratio - 3.8*ratio*ratio + 520.0*ratio + 0.62
}

// kernelNowNs reads the clock the kernel stamps GPIO edge events with
// (CLOCK_MONOTONIC), so a pulse spanning a window boundary can be split
// in the events' own time base.
func kernelNowNs() uint64 {
	var ts unix.Timespec
	if err := unix.ClockGettime(unix.CLOCK_MONOTONIC, &ts); err != nil {
		return 0
	}
	return uint64(ts.Nano())
}

// ppd42ns accumulates PPD42NS low-pulse occupancy into a dust
// concentration. A goroutine drains the kernel's edge events; read()
// lazily closes the LPO window once it is due.
type ppd42ns struct {
	line *common.GpioLine

	mu          sync.Mutex
	lpoNs       uint64 // nanoseconds of low time this window
	lowSinceNs  uint64 // kernel timestamp of the current falling edge
	inLow       bool
	windowStart time.Time
	result      *float64 // nil until the first window closes
}

// newPpd42ns requests the pin as an edge-event input. The PPD42NS drives
// its output actively (through the divider), so no internal bias is set.
func newPpd42ns(chipPath string, pin int) (*ppd42ns, error) {
	line, err := common.OpenEventLine(chipPath, pin, false, false, false)
	if err != nil {
		return nil, err
	}
	s := &ppd42ns{line: line, windowStart: time.Now()}
	// Edge events never fire for a pin that is already low (a stuck or
	// misbehaving line), which would read as 0 % occupancy — "perfectly
	// clean air" — instead of saturation. Treat an initial low as a pulse
	// in progress so a stuck-low line reports a huge value, not a clean one.
	if high, err := line.Get(); err == nil && !high {
		s.inLow = true
		s.lowSinceNs = kernelNowNs()
	}
	go s.watch()
	return s, nil
}

// watch accumulates low time from the kernel's edge timestamps: falling
// edge starts a pulse, rising edge credits it to the current window.
func (s *ppd42ns) watch() {
	for {
		event, err := s.line.ReadEvent(time.Second)
		if err != nil {
			fmt.Println("GPIO event read failed:", err)
			time.Sleep(time.Second)
			continue
		}
		if event == nil {
			continue // timeout — no edge within a second
		}
		s.mu.Lock()
		if event.Rising {
			if s.inLow && event.TimestampNs > s.lowSinceNs {
				s.lpoNs += event.TimestampNs - s.lowSinceNs
			}
			s.inLow = false
		} else {
			s.lowSinceNs = event.TimestampNs
			s.inLow = true
		}
		s.mu.Unlock()
	}
}

// maybeRoll closes the LPO window once it is due and caches the result.
//
// Called lazily from read(), so the window grows past windowSeconds when
// nobody polls — harmless, because the ratio divides by the *actual*
// elapsed time rather than the nominal window length.
func (s *ppd42ns) maybeRoll() {
	now := time.Now()
	s.mu.Lock()
	defer s.mu.Unlock()
	elapsed := now.Sub(s.windowStart).Seconds()
	if elapsed < windowSeconds {
		return
	}
	lpoNs := s.lpoNs
	if s.inLow {
		// A pulse spans the boundary: credit the elapsed part to the
		// closing window and restart the timer for the new one.
		kernelNow := kernelNowNs()
		if kernelNow > s.lowSinceNs {
			lpoNs += kernelNow - s.lowSinceNs
		}
		s.lowSinceNs = kernelNow
	}
	s.lpoNs = 0
	s.windowStart = now
	ratio := float64(lpoNs) / 1e9 / elapsed * 100.0
	c := concentration(ratio)
	s.result = &c
}

// read returns the full-key REST payload, or nil while warming up.
func (s *ppd42ns) read() map[string]any {
	s.maybeRoll()
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.result == nil {
		return nil
	}
	return map[string]any{"dust": round1(*s.result)}
}

// -- Main --------------------------------------------------------------------

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	Pin       int    `json:"pin"`
	GpioChip  int    `json:"gpio_chip"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
	SSLCert   string `json:"ssl_cert"`
	SSLKey    string `json:"ssl_key"`
}

// read returns the full-key REST payload, or nil while warming up.
type readFunc func() map[string]any

// emulatedReader generates plausible readings without hardware: indoor
// air over the day — the concentration follows a slow sine between
// roughly 100 and 700 pcs/0.01cf, deliberately crossing several Dylos
// air-quality bands, with a little measurement jitter, never below zero
// (like the Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		c := 400.0 + 300.0*math.Sin(t/180.0) + uniform(-40, 40)
		return map[string]any{"dust": round1(math.Max(0.0, c))}
	}
}

// discoveryFor returns short-key readings for the UDP discovery response.
// The dust value uses the same key on both payloads; during warm-up the
// reply falls back to identity-only (never nil, which would drop it).
func discoveryFor(read readFunc) func() map[string]any {
	return func() map[string]any {
		full := read()
		if full == nil {
			return map[string]any{}
		}
		return full
	}
}

func main() {
	cfg := Config{Pin: 17, Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var read readFunc
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating PPD42NS readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing Grove Dust Sensor (PPD42NS) on GPIO %d...\n", cfg.Pin)
		fmt.Println("First reading after the first full 30-second LPO window.")
		sensor, err := newPpd42ns(fmt.Sprintf("/dev/gpiochip%d", cfg.GpioChip), cfg.Pin)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = sensor.read
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener("PPD42NS", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("PPD42NS", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
