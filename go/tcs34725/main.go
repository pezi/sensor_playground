// Sensor Playground Sensor Node — TCS34725 (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a TCS34725 I2C sensor (RGB colour,
// colour temperature, illuminance).
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
//	./sensor_node_tcs34725
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"time"

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

// read returns the full-key REST payload, or nil on a failed or saturated
// read.
type readFunc func() map[string]any

func realReader(s *TCS34725) readFunc {
	return func() map[string]any {
		reading, ok, err := s.Read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		if !ok {
			// The clear channel saturated: there is no colour to report.
			return nil
		}
		return reading
	}
}

// hsvToRGB converts a hue/saturation/value triple to red, green and blue in
// 0..1 — the same sextant maths as Python's colorsys.hsv_to_rgb, used only
// to drive the emulated colour cycle.
func hsvToRGB(h, s, v float64) (float64, float64, float64) {
	if s == 0.0 {
		return v, v, v
	}
	sextant := int(h * 6.0)
	f := h*6.0 - float64(sextant)
	p := v * (1.0 - s)
	q := v * (1.0 - s*f)
	t := v * (1.0 - s*(1.0-f))
	switch sextant % 6 {
	case 0:
		return v, t, p
	case 1:
		return q, v, p
	case 2:
		return p, v, t
	case 3:
		return p, q, v
	case 4:
		return t, p, v
	default:
		return v, p, q
	}
}

// emulatedReader generates plausible readings without hardware: a coloured
// light slowly cycling through the hue circle every 30 seconds, with the
// colour temperature swinging between warm and cool white and the
// illuminance drifting around a few hundred lux (like the Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		r, g, b := hsvToRGB(math.Mod(t/30.0, 1.0), 0.6, 0.9)
		return map[string]any{
			"colorTemperature": int(math.Round(4600 + 1900*math.Sin(t/45.0))),
			"lux":              int(math.Round(300 + 200*math.Sin(t/75.0) + uniform(-5, 5))),
			"red":              int(math.Round(r * 255)),
			"green":            int(math.Round(g * 255)),
			"blue":             int(math.Round(b * 255)),
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
			"ct":  full["colorTemperature"],
			"lux": full["lux"],
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
		fmt.Println("Emulation mode: generating TCS34725 readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing TCS34725 sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		sensor, err := NewTCS34725(cfg.I2CBus)
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
		err := common.RunDiscoveryListener("TCS34725", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("TCS34725", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
