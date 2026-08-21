// Sensor Playground Sensor Node — Grove Ultrasonic Ranger (Go)
//
// Implements the *push* variant of the Sensor Playground Sensor Interface
// on single-board computers (Raspberry Pi & co.) with a Grove Ultrasonic
// Ranger (40 kHz sonar, 2 cm - 3.5 m). Like the VL53L0X node it measures
// continuously and pushes one JSON message ({"distance": <mm>}, null when
// no echo returns) whenever the distance changes, or at least once per
// second as a heartbeat.
//
// The Grove ranger uses a single SIG pin for both trigger and echo —
// unlike the common HC-SR04 with separate TRIG/ECHO pins. One measurement:
// drive SIG with a trigger pulse, switch the pin to input, and time the
// echo pulse the module answers with; the pulse width divided by twice the
// speed of sound is the distance.
//
// The echo is timed via kernel-timestamped GPIO edge events (character
// device, see ../common) — a userspace polling loop would add milliseconds
// of jitter, and one millisecond of pulse error is 17 cm of distance
// error. (The trigger pulse only has a minimum width, so time.Sleep's
// overshoot is harmless there.) There is no extension-hat option: the
// Arduino-based hats are polled over I2C and cannot time the echo.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to generate plausible readings
// without the sensor hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_ultrasonic
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"time"

	"sensorplayground/common"
)

// -- Publish policy -----------------------------------------------------------

const (
	measureInterval = 100 * time.Millisecond
	heartbeat       = time.Second
	// Larger than the VL53L0X's delta because a sonar reading jitters a little.
	minDeltaMM = 5
)

// -- Measurement --------------------------------------------------------------

const (
	// Sound travels 0.343 mm/µs = 343 mm per million ns; the echo pulse
	// covers the distance twice.
	mmPerNs = 0.343e-3 / 2.0

	// The echo of a 3.5 m target takes ~20 ms; give up shortly after that.
	echoTimeout = 60 * time.Millisecond

	minRangeMM = 20
	maxRangeMM = 3500

	// Trigger pulse: >=10 µs high. time.Sleep overshoots, which the module
	// tolerates — the pulse only has a minimum width.
	triggerPulse = 20 * time.Microsecond
)

// pulseToMM converts an echo pulse width [ns] to a distance in mm, or nil
// when the result is outside the ranger's 2 cm - 3.5 m range.
func pulseToMM(pulseNs uint64) *int {
	mm := int(math.Round(float64(pulseNs) * mmPerNs))
	if mm < minRangeMM || mm > maxRangeMM {
		return nil
	}
	return &mm
}

// Ranger returns the distance in mm, or nil without an echo in range.
type Ranger interface {
	ReadMM() (*int, error)
}

// gpioRanger times the ranger's single-wire trigger/echo cycle: the SIG
// line is requested as output for the trigger pulse, then re-requested as
// an edge-event input for the echo — the same mode flips the Python node
// does via lgpio claims.
type gpioRanger struct {
	chipPath string
	pin      int
}

// newGpioRanger probes the line once so a wrong chip/pin (or a non-Linux
// host) fails at startup instead of on every measurement.
func newGpioRanger(chipPath string, pin int) (*gpioRanger, error) {
	line, err := common.OpenEventLine(chipPath, pin, false, false, false)
	if err != nil {
		return nil, err
	}
	line.Close()
	return &gpioRanger{chipPath: chipPath, pin: pin}, nil
}

func (r *gpioRanger) ReadMM() (*int, error) {
	// Trigger: a >=10 µs high pulse.
	out, err := common.OpenOutputLine(r.chipPath, r.pin, false)
	if err != nil {
		return nil, err
	}
	time.Sleep(triggerPulse)
	out.Set(true)
	time.Sleep(triggerPulse)
	out.Set(false)
	out.Close()

	// Echo: hand the pin to the kernel's edge machinery and wait for the
	// pulse, timing it from the kernel's edge timestamps.
	line, err := common.OpenEventLine(r.chipPath, r.pin, false, false, false)
	if err != nil {
		return nil, err
	}
	defer line.Close()

	deadline := time.Now().Add(echoTimeout)
	var riseNs uint64
	haveRise := false
	for {
		remaining := time.Until(deadline)
		if remaining <= 0 {
			return nil, nil // no echo
		}
		event, err := line.ReadEvent(remaining)
		if err != nil {
			return nil, err
		}
		if event == nil {
			return nil, nil // no echo
		}
		if event.Rising {
			riseNs = event.TimestampNs
			haveRise = true
		} else if haveRise {
			return pulseToMM(event.TimestampNs - riseNs), nil
		}
	}
}

// emulatedRanger generates plausible ranger readings without hardware: a
// target sweeping back and forth between 200 and 2000 mm (20 s period),
// occasionally leaving the measuring range.
type emulatedRanger struct{}

func (emulatedRanger) ReadMM() (*int, error) {
	if rand.Float64() < 0.02 {
		return nil, nil
	}
	phase := math.Mod(float64(time.Now().UnixNano())/1e9, 20.0) / 20.0
	mm := int(math.Round(200 + 1800*(1-math.Abs(2*phase-1))))
	return &mm, nil
}

// -- Measure loop --------------------------------------------------------------

// shouldPublish implements the push policy: the first reading, an
// echo/no-echo flip, or a change of at least minDeltaMM publish
// immediately (heartbeat aside).
func shouldPublish(everPublished bool, lastSent, mm *int) bool {
	if !everPublished {
		return true
	}
	if (mm == nil) != (lastSent == nil) {
		return true
	}
	if mm == nil || lastSent == nil {
		return false
	}
	delta := *mm - *lastSent
	if delta < 0 {
		delta = -delta
	}
	return delta >= minDeltaMM
}

// measureLoop measures continuously; publishes on change, range flip, or
// heartbeat.
func measureLoop(sensor Ranger, publish func(payload any)) {
	var lastSent *int
	everPublished := false
	var lastPublish time.Time

	for {
		mm, err := sensor.ReadMM()
		if err != nil {
			fmt.Println("GPIO read failed:", err)
			time.Sleep(measureInterval)
			continue
		}
		now := time.Now()
		if shouldPublish(everPublished, lastSent, mm) || now.Sub(lastPublish) >= heartbeat {
			var distance any
			if mm != nil {
				distance = *mm
			}
			publish(map[string]any{"distance": distance})
			everPublished = true
			lastSent = mm
			lastPublish = now
		}
		time.Sleep(measureInterval)
	}
}

// -- Main --------------------------------------------------------------------

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	SigPin     int    `json:"sig_pin"`
	GpioChip   int    `json:"gpio_chip"`
	SensorName string `json:"sensor_name"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

func main() {
	cfg := Config{SigPin: 4, SensorName: "ULTRASONIC", Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var sensor Ranger
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating ranger readings without hardware")
		sensor = emulatedRanger{}
	} else {
		fmt.Printf("Initializing ultrasonic ranger on GPIO %d...\n", cfg.SigPin)
		ranger, err := newGpioRanger(fmt.Sprintf("/dev/gpiochip%d", cfg.GpioChip), cfg.SigPin)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		sensor = ranger
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey, nil, nil)

	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go measureLoop(sensor, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
