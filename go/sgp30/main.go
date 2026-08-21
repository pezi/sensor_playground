// Sensor Playground Sensor Node — SGP30 (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a SGP30 I2C sensor (eCO2, TVOC).
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
//	./sensor_node_sgp30
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"

	"sensorplayground/common"
)

func uniform(lo, hi float64) float64 { return lo + rand.Float64()*(hi-lo) }

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	I2CBus    int    `json:"i2c_bus"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
	SSLCert   string `json:"ssl_cert"`
	SSLKey    string `json:"ssl_key"`
}

// read returns the full-key REST payload, or nil on a failed read.
type readFunc func() map[string]any

func realReader(s *SGP30) readFunc {
	return func() map[string]any {
		eco2, tvoc, err := s.Read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		return map[string]any{
			"eco2": int(eco2),
			"tvoc": int(tvoc),
		}
	}
}

// emulatedReader generates plausible readings without hardware: indoor air
// quality drifting around typical office values — eCO2 does a bounded
// random walk between 400 and 1500 ppm, TVOC between 0 and 600 ppb (like
// the Python node).
func emulatedReader() readFunc {
	eco2, tvoc := 600.0, 60.0
	return func() map[string]any {
		eco2 = math.Min(math.Max(eco2+uniform(-15, 15), 400.0), 1500.0)
		tvoc = math.Min(math.Max(tvoc+uniform(-8, 8), 0.0), 600.0)
		return map[string]any{
			"eco2": int(math.Round(eco2)),
			"tvoc": int(math.Round(tvoc)),
		}
	}
}

// The Python node's read_discovery returns the same full keys as read.
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
	cfg := Config{I2CBus: 1, Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
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
		fmt.Println("Emulation mode: generating SGP30 readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing SGP30 sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		sensor, err := NewSGP30(cfg.I2CBus)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		fmt.Println("Warming up SGP30 (about 15 seconds)...")
		if err := sensor.StartMeasurement(); err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = realReader(sensor)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener("SGP30", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("SGP30", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
