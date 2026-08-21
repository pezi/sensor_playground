// Sensor Playground Sensor Node — DHT11 / Grove Temperature & Humidity (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a DHT11 sensor (temperature,
// humidity) — the blue Grove Temperature & Humidity Sensor module. The
// DHT22 (Grove "Pro" module, white) speaks the same single-wire protocol
// with better resolution and has its own node in ../dht22/. Setting
// "sensor_name": "DHT22" here remains supported for older configurations.
//
// The single-wire protocol is timing-critical (26-70 µs pulses), so the
// pin is read via kernel-timestamped GPIO edge events (character device,
// see ../common) — a userspace polling loop could never tell the pulse
// widths apart. Reads occasionally fail even on a healthy sensor; the
// node retries and serves the last good reading for up to 30 seconds, so
// a single failed read does not surface as an error.
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
//	./sensor_node_dht11
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
	GpioPin    int    `json:"gpio_pin"`
	GpioChip   int    `json:"gpio_chip"`
	SensorName string `json:"sensor_name"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
	SSLCert    string `json:"ssl_cert"`
	SSLKey     string `json:"ssl_key"`
}

// read returns the full-key REST payload, or nil on a failed read.
type readFunc func() map[string]any

// emulatedReader generates plausible DHT11 readings without hardware: a
// comfortable indoor climate drifting on slow sines around 22 °C / 45 %RH,
// quantised to the DHT11's whole-degree / whole-percent resolution (like
// the Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		return map[string]any{
			"temperature": math.Round(22.0 + 2.0*math.Sin(t/60.0)),
			"humidity":    math.Round(45.0 + 8.0*math.Sin(t/97.0) + uniform(-0.5, 0.5)),
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
	cfg := Config{GpioPin: 4, SensorName: "DHT11", Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
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
		fmt.Printf("Initializing %s sensor on GPIO %d...\n", cfg.SensorName, cfg.GpioPin)
		sensor, err := newDhtSensor(fmt.Sprintf("/dev/gpiochip%d", cfg.GpioChip), cfg.GpioPin, cfg.SensorName)
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
