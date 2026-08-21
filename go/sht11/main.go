// Sensor Playground Sensor Node — SHT11 / Sensirion SHT1x (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a Sensirion SHT1x sensor
// (temperature, humidity) — the classic SHT10 / SHT11 / SHT15 family.
// The chips differ only in calibration accuracy and speak the same
// proprietary two-wire protocol (SCK + bidirectional DATA); it resembles
// I2C but is NOT I2C — the sensor cannot share an I2C bus. Set
// "sensor_name" in config.json to the chip on your board so the app
// shows the right name.
//
// The protocol is bit-banged on two GPIOs (see sht11.go): the bus is
// fully master-clocked with no minimum speed, so the pace of one GPIO
// call per clock edge is harmless — the sensor simply waits between
// edges (unlike the DHT11, whose reply timing must be captured by the
// kernel). The sensor must not be measured more than ~10% of the time
// or it heats itself; the node reads at most every two seconds and
// serves the cached values, and a failed read serves the last good
// reading for up to 30 seconds before read() reports a failure.
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
//	./sensor_node_sht11
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
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	DataPin    int    `json:"data_pin"`
	SckPin     int    `json:"sck_pin"`
	GpioChip   int    `json:"gpio_chip"`
	SensorName string `json:"sensor_name"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
	SSLCert    string `json:"ssl_cert"`
	SSLKey     string `json:"ssl_key"`
}

// read returns the full-key REST payload, or nil on a failed read.
type readFunc func() map[string]any

// emulatedReader generates plausible readings without hardware: a
// comfortable indoor climate drifting on slow sines (like the Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		return map[string]any{
			"temperature": round1(22.0 + 2.0*math.Sin(t/60.0) + uniform(-0.1, 0.1)),
			"humidity":    round1(45.0 + 8.0*math.Sin(t/97.0) + uniform(-0.5, 0.5)),
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
		}
	}
}

func main() {
	cfg := Config{DataPin: 4, SckPin: 5, SensorName: "SHT11", Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
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
		fmt.Printf("Emulation mode: generating %s readings without hardware\n", cfg.SensorName)
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing %s sensor (DATA=GPIO %d, SCK=GPIO %d)...\n",
			cfg.SensorName, cfg.DataPin, cfg.SckPin)
		sensor, err := NewSHT1x(fmt.Sprintf("/dev/gpiochip%d", cfg.GpioChip), cfg.DataPin, cfg.SckPin)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = sensor.Read
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer(cfg.SensorName, cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
