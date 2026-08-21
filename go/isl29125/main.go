// Sensor Playground Sensor Node — SparkFun ISL29125 RGB Light Sensor (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers with a SparkFun RGB Light Sensor breakout — an
// Intersil/Renesas ISL29125 (I2C address 0x44) measuring the intensity of
// red, green and blue light while rejecting infrared. The channels are
// normalized against the brightest one so the app can show the measured
// colour directly (JSON keys red/green/blue, 0-255, absent in complete
// darkness); an approximate illuminance (lux) is derived from the green
// channel. There is no clear channel, so unlike the TCS34725 no colour
// temperature is reported.
// https://www.sparkfun.com/sparkfun-rgb-light-sensor-isl29125.html
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
//	./sensor_node_isl29125
package main

import (
	"fmt"
	"math"
	"os"
	"time"

	"sensorplayground/common"
)

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	I2CBus    int    `json:"i2c_bus"`
	Address   int    `json:"address"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
	SSLCert   string `json:"ssl_cert"`
	SSLKey    string `json:"ssl_key"`
}

// readChannelsFunc returns the raw 16-bit green, red and blue counts, or
// an error on a failed read.
type readChannelsFunc func() (green, red, blue uint16, err error)

// emulatedChannels generates plausible readings without hardware: indoor
// light slowly shifting between warm and cool white, with the brightness
// breathing over a couple of minutes (like the Python node).
func emulatedChannels() readChannelsFunc {
	return func() (uint16, uint16, uint16, error) {
		t := float64(time.Now().UnixNano()) / 1e9
		brightness := 0.35 + 0.3*math.Sin(t/120.0)
		warmth := 0.5 + 0.5*math.Sin(t/45.0)
		counts := func(x float64) uint16 { return uint16(math.Round(65535 * x)) }
		return counts(brightness),
			counts(brightness * (0.6 + 0.4*warmth)),
			counts(brightness * (1.0 - 0.5*warmth)),
			nil
	}
}

func main() {
	cfg := Config{I2CBus: 1, Address: 0x44, Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var readChannels readChannelsFunc
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating ISL29125 readings without hardware")
		readChannels = emulatedChannels()
	} else {
		fmt.Printf("Initializing ISL29125 on I2C bus %d, address 0x%02x...\n", cfg.I2CBus, cfg.Address)
		sensor, err := NewISL29125(cfg.I2CBus, uint8(cfg.Address))
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		readChannels = sensor.ReadChannels
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	// The REST payload carries the colour, the discovery reply only the
	// illuminance — as in the Python node.
	read := func() map[string]any {
		green, red, blue, err := readChannels()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		return deriveReading(green, red, blue)
	}
	readDiscovery := func() map[string]any {
		green, _, _, err := readChannels()
		if err != nil {
			return map[string]any{}
		}
		return map[string]any{"lux": int(math.Round(float64(green) * luxPerCount))}
	}

	go func() {
		err := common.RunDiscoveryListener("ISL29125", cfg.Hostname, common.HTTPSPort, readDiscovery)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("ISL29125", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
