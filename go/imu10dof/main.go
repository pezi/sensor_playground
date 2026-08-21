// Sensor Playground Sensor Node — Grove IMU 10DOF (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a Grove IMU 10DOF board
// (MPU9250 + BMP280). The MPU9250 axes are reduced to roll / pitch /
// compass heading angles plus the total acceleration magnitude (g-force);
// the BMP280 adds temperature and barometric pressure.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//     — the app streams motion sensors rather than polling them
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to generate plausible readings
// without the sensor hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_imu10dof
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"time"

	"sensorplayground/common"
)

const sensorName = "IMU10DOF"

// The app streams motion sensors instead of polling them, so readings are
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

// read returns the full-key live payload, or nil on a failed read.
type readFunc func() map[string]any

func realReader(s *IMU10DOF) readFunc {
	return func() map[string]any {
		r, err := s.Read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		return map[string]any{
			"temperature": round1(r.Temperature),
			"pressure":    round2(r.Pressure),
			"roll":        round1(r.Roll),
			"pitch":       round1(r.Pitch),
			"heading":     round1(r.Heading),
			"gforce":      round2(r.GForce),
		}
	}
}

// emulatedReader generates plausible readings without hardware: a gently
// rocking, near-level board — roll and pitch follow slow sines with
// different periods, the g-force stays around 1 g and the heading sweeps
// a full circle every two minutes. The BMP280 side reports room
// temperature and sea-level pressure, each drifting slowly (like the
// Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		return map[string]any{
			"temperature": round1(22.0 + 2.0*math.Sin(t/60.0)),
			"pressure":    round2(1013.0 + 3.0*math.Sin(t/300.0)),
			"roll":        round1(8.0*math.Sin(t/7.0) + uniform(-0.3, 0.3)),
			"pitch":       round1(5.0*math.Sin(t/11.0) + uniform(-0.3, 0.3)),
			"heading":     round1(math.Mod(t*3, 360)),
			"gforce":      round2(1.0 + uniform(-0.02, 0.02)),
		}
	}
}

func discoveryFor(read readFunc) func() map[string]any {
	return func() map[string]any {
		full := read()
		if full == nil {
			return map[string]any{}
		}
		return map[string]any{
			"temp":    full["temperature"],
			"press":   full["pressure"],
			"roll":    full["roll"],
			"pitch":   full["pitch"],
			"heading": full["heading"],
			"gforce":  full["gforce"],
		}
	}
}

// pushLoop pushes a reading every pushInterval, like the ESP32 sketch.
func pushLoop(read readFunc, hostname string, publish func(payload any)) {
	for {
		if data := read(); data != nil {
			payload := map[string]any{"sensor": sensorName, "host": hostname}
			for k, v := range data {
				payload[k] = v
			}
			publish(payload)
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
		fmt.Println("Emulation mode: generating IMU10DOF readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing IMU 10DOF on /dev/i2c-%d...\n", cfg.I2CBus)
		sensor, err := NewIMU10DOF(cfg.I2CBus)
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
		err := common.RunDiscoveryListener(sensorName, cfg.Hostname, common.WSPort, discoveryFor(read))
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
