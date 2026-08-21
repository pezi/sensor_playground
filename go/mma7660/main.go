// Sensor Playground Sensor Node — MMA7660 (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a Grove 3-Axis Digital Accelerometer
// ±1.5g (MMA7660FC). The raw axes are converted into roll / pitch angles
// plus the total acceleration magnitude (g-force).
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133 —
//     the app streams accelerometers rather than polling them
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to generate plausible readings
// without the sensor hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_mma7660
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"time"

	"sensorplayground/common"
)

// The app streams accelerometers instead of polling them, so readings are
// pushed at the same 250 ms cadence the BLE transport and the ESP32 use.
const pushInterval = 250 * time.Millisecond

func round1(x float64) float64 { return math.Round(x*10) / 10 }
func round2(x float64) float64 { return math.Round(x*100) / 100 }

func uniform(lo, hi float64) float64 { return lo + rand.Float64()*(hi-lo) }

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	I2CBus    int    `json:"i2c_bus"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
}

// read returns the full-key payload, or nil on a failed read.
type readFunc func() map[string]any

func realReader(s *MMA7660) readFunc {
	return func() map[string]any {
		r, err := s.Read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		return map[string]any{
			"roll":   round1(r.Roll),
			"pitch":  round1(r.Pitch),
			"gforce": round2(r.GForce),
		}
	}
}

// emulatedReader generates plausible readings without hardware: a gently
// rocking, near-level board — roll and pitch follow slow sines with
// different periods and the g-force stays around 1 g (like the Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		return map[string]any{
			"roll":   round1(8.0*math.Sin(t/7.0) + uniform(-0.3, 0.3)),
			"pitch":  round1(5.0*math.Sin(t/11.0) + uniform(-0.3, 0.3)),
			"gforce": round2(1.0 + uniform(-0.02, 0.02)),
		}
	}
}

// discoveryFor returns the discovery reading — the same full keys as the
// WebSocket payload (the Python node reuses read() for discovery) — or an
// identity-only reply on a failed read.
func discoveryFor(read readFunc) func() map[string]any {
	return func() map[string]any {
		full := read()
		if full == nil {
			return map[string]any{}
		}
		return full
	}
}

// pushLoop pushes a reading every pushInterval, like the ESP32 sketch.
func pushLoop(read readFunc, hostname string, broadcast func(payload any)) {
	for {
		if data := read(); data != nil {
			payload := map[string]any{"sensor": "MMA7660", "host": hostname}
			for k, v := range data {
				payload[k] = v
			}
			broadcast(payload)
		}
		time.Sleep(pushInterval)
	}
}

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
		fmt.Println("Emulation mode: generating MMA7660 readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing MMA7660 sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		sensor, err := NewMMA7660(cfg.I2CBus)
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
		err := common.RunDiscoveryListener("MMA7660", cfg.Hostname, common.WSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go pushLoop(read, cfg.Hostname, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
