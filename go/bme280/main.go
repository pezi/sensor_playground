// Sensor Playground Sensor Node — BME280 (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a BME280 I2C sensor (temperature,
// humidity, pressure).
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
//	./sensor_node_bme280
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"time"

	"sensorplayground/common"
)

func round1(x float64) float64 { return math.Round(x*10) / 10 }
func round2(x float64) float64 { return math.Round(x*100) / 100 }

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

func realReader(s *BME280) readFunc {
	return func() map[string]any {
		r, err := s.Read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		return map[string]any{
			"temperature": round1(r.Temperature),
			"humidity":    round1(r.Humidity),
			"pressure":    round2(r.Pressure),
		}
	}
}

// emulatedReader generates plausible readings without hardware: a
// comfortable indoor climate drifting on slow sines (like the Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		return map[string]any{
			"temperature": round1(22.0 + 2.0*math.Sin(t/60.0) + uniform(-0.1, 0.1)),
			"humidity":    round1(45.0 + 8.0*math.Sin(t/97.0) + uniform(-0.5, 0.5)),
			"pressure":    round2(1013.0 + 3.0*math.Sin(t/300.0) + uniform(-0.2, 0.2)),
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
			"temp":  full["temperature"],
			"hum":   full["humidity"],
			"press": full["pressure"],
		}
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
		fmt.Println("Emulation mode: generating BME280 readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing BME280 sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		sensor, err := NewBME280(cfg.I2CBus)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = realReader(sensor)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener("BME280", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("BME280", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
