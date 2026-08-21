// Sensor Playground Sensor Node — SCD30 (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a SCD30 I2C sensor (temperature,
// humidity, CO2).
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
//	./sensor_node_scd30
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

func realReader(s *SCD30) readFunc {
	return func() map[string]any {
		co2, temperature, humidity, ok, err := s.Read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		if !ok {
			// No fresh measurement yet (2 s interval), like the Python
			// node's read() returning None.
			return nil
		}
		return map[string]any{
			"temperature": round1(temperature),
			"humidity":    round1(humidity),
			"co2":         round1(co2),
		}
	}
}

// emulatedReader generates plausible readings without hardware: a quiet
// indoor room — temperature and humidity follow slow sines with different
// periods, the CO2 concentration does a bounded random walk between 400
// and 1500 ppm (like the Python node).
func emulatedReader() readFunc {
	co2 := 600.0
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		co2 = math.Min(math.Max(co2+uniform(-15, 15), 400.0), 1500.0)
		return map[string]any{
			"temperature": round1(22.0 + 2.0*math.Sin(t/60.0) + uniform(-0.1, 0.1)),
			"humidity":    round1(45.0 + 8.0*math.Sin(t/97.0) + uniform(-0.5, 0.5)),
			"co2":         round1(co2),
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
			"temp": full["temperature"],
			"hum":  full["humidity"],
			"co2":  full["co2"],
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
		fmt.Println("Emulation mode: generating SCD30 readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing SCD30 sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		sensor, err := NewSCD30(cfg.I2CBus)
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
		err := common.RunDiscoveryListener("SCD30", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("SCD30", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
