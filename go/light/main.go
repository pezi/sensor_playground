// Sensor Playground Sensor Node — Grove Light Sensor (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers with a Grove Light Sensor — an analog photo-resistor reporting
// a raw brightness value (higher = brighter) under the JSON key `light`.
//
// The Raspberry Pi has no analog input, so the sensor is read through the
// Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04, one 16-bit register
// per channel). The Arduino-based hats the Python node also supports
// ("nano", "grovePlus") are not implemented in this port.
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
//	./sensor_node_light
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"time"

	"sensorplayground/common"
)

const (
	hatI2CAddress = 0x04 // Grove Base Hat (STM32F030 ADC)
	hatADCBase    = 0x10 // raw 12-bit value registers, one per channel
)

func uniform(lo, hi float64) float64 { return lo + rand.Float64()*(hi-lo) }

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	HatType   string `json:"hat_type"`
	Pin       int    `json:"pin"`
	I2CBus    int    `json:"i2c_bus"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
	SSLCert   string `json:"ssl_cert"`
	SSLKey    string `json:"ssl_key"`
}

type readFunc func() map[string]any

// realReader reads the raw 12-bit ADC value [0-4095] of the channel.
func realReader(dev *common.I2CDevice, pin int) readFunc {
	return func() map[string]any {
		data, err := dev.ReadRegs(uint8(hatADCBase+pin), 2)
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		value := int(data[0]) | int(data[1])<<8 // SMBus words are little-endian
		return map[string]any{"light": value}
	}
}

// emulatedReader: daylight through a window — a slow sine around a few
// hundred counts with a little flicker, never below zero.
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		raw := 400 + 350*math.Sin(t/120.0) + uniform(-15, 15)
		return map[string]any{"light": int(math.Max(0, math.Round(raw)))}
	}
}

func main() {
	cfg := Config{HatType: "grove", I2CBus: 1, Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
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
		fmt.Println("Emulation mode: generating LIGHT readings without hardware")
		read = emulatedReader()
	} else {
		if cfg.HatType != "grove" {
			fmt.Printf("Error: hat_type %q is not supported in the Go port (only \"grove\"; use the Python node for Arduino-based hats)\n", cfg.HatType)
			os.Exit(1)
		}
		if cfg.Pin < 0 || cfg.Pin > 7 {
			fmt.Printf("Error: invalid channel %d - valid range [0,7]\n", cfg.Pin)
			os.Exit(1)
		}
		fmt.Printf("Initializing Grove Light Sensor on grove hat, channel %d...\n", cfg.Pin)
		dev, err := common.OpenI2C(cfg.I2CBus, hatI2CAddress)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = realReader(dev, cfg.Pin)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		// The light value uses the same key on both payloads.
		err := common.RunDiscoveryListener("LIGHT", cfg.Hostname, common.HTTPSPort,
			func() map[string]any {
				if full := read(); full != nil {
					return full
				}
				return map[string]any{}
			})
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("LIGHT", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
