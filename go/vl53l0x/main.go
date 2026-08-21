// Sensor Playground Sensor Node — VL53L0X (Go)
//
// Implements the *push* variant of the Sensor Playground Sensor Interface
// on single-board computers (Raspberry Pi & co.) with a VL53L0X
// time-of-flight distance sensor. The node measures continuously and
// pushes one JSON message ({"distance": <mm>}, null when out of range)
// whenever the distance changes, or at least once per second as a
// heartbeat.
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
//	./sensor_node_vl53l0x
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
	minDeltaMM      = 3
)

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	I2CBus    int    `json:"i2c_bus"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
}

// -- Sensor ------------------------------------------------------------------

// read returns the distance in mm, or nil when no target is in range.
type readFunc func() (*int, error)

func realReader(s *VL53L0X) readFunc {
	return func() (*int, error) {
		mm, err := s.Range()
		if err != nil {
			return nil, err
		}
		// The VL53L0X reports ~8190 mm when no target is in range.
		if mm > 0 && mm < 8000 {
			return &mm, nil
		}
		return nil, nil
	}
}

// emulatedReader generates plausible VL53L0X readings without hardware: a
// target sweeping back and forth between 100 and 1200 mm (20 s period),
// occasionally leaving the measuring range (like the Python node).
func emulatedReader() readFunc {
	return func() (*int, error) {
		if rand.Float64() < 0.02 {
			return nil, nil
		}
		phase := math.Mod(float64(time.Now().UnixNano())/1e9, 20.0) / 20.0
		mm := int(math.Round(100 + 1100*(1-math.Abs(2*phase-1))))
		return &mm, nil
	}
}

// -- Measure loop -------------------------------------------------------------

// measureLoop measures continuously; publishes on change, range flip, or
// heartbeat.
func measureLoop(read readFunc, publish func(payload any)) {
	var lastSent *int
	var lastPublish time.Time
	everPublished := false

	for {
		mm, err := read()
		if err != nil {
			fmt.Println("I2C read failed:", err)
			time.Sleep(measureInterval)
			continue
		}
		changed := !everPublished ||
			(mm == nil) != (lastSent == nil) ||
			(mm != nil && lastSent != nil && abs(*mm-*lastSent) >= minDeltaMM)
		if changed || time.Since(lastPublish) >= heartbeat {
			publish(map[string]any{"distance": mm})
			everPublished = true
			lastSent = mm
			lastPublish = time.Now()
		}
		time.Sleep(measureInterval)
	}
}

func abs(x int) int {
	if x < 0 {
		return -x
	}
	return x
}

// -- Main --------------------------------------------------------------------

func main() {
	cfg := Config{I2CBus: 1, Transport: "wifi"}
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
		fmt.Println("Emulation mode: generating VL53L0X readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Println("Initializing VL53L0X sensor...")
		sensor, err := NewVL53L0X(cfg.I2CBus)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = realReader(sensor)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey, nil, nil)

	go func() {
		err := common.RunDiscoveryListener("VL53L0X", cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go measureLoop(read, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
