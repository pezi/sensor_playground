// Sensor Playground Sensor Node — Grove Capacitive Moisture Sensor (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers with a Grove Capacitive Moisture Sensor (Corrosion-Resistant)
// — an analog probe whose output voltage falls as the soil gets wetter.
// It reports the soil moisture as a percentage (JSON key `moisture`)
// mapped linearly between the two calibration points in config.json (raw
// ADC when dry vs. when wet), alongside the raw reading (`adc`/`adcMax`)
// for calibrating them.
//
// The Raspberry Pi has no analog input, so the probe is read through the
// Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04, one 16-bit
// register per channel). The Arduino-based hats the Python node also
// supports ("nano", "grovePlus") are not implemented in this port.
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
//	./sensor_node_moisture
package main

import (
	"fmt"
	"math"
	"os"
	"time"

	"sensorplayground/common"
)

const (
	hatI2CAddress = 0x04 // Grove Base Hat (STM32F030 ADC)
	hatADCBase    = 0x10 // raw 12-bit value registers, one per channel
	hatADCMax     = 4095 // full scale of the hat's 12-bit ADC

	// Averaging window; a single ADC read of the probe is noisy.
	sampleCount = 4
)

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	HatType   string `json:"hat_type"`
	Pin       int    `json:"pin"`
	I2CBus    int    `json:"i2c_bus"`
	AdcDry    int    `json:"adc_dry"`
	AdcWet    int    `json:"adc_wet"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
	SSLCert   string `json:"ssl_cert"`
	SSLKey    string `json:"ssl_key"`
}

// rawFunc returns the averaged raw ADC reading, or false when a read failed.
type rawFunc func() (int, bool)

// percent maps a raw ADC reading onto 0-100 % between the dry and wet
// calibration points (the probe's output falls as the soil gets wetter).
func percent(raw, adcDry, adcWet int) int {
	span := float64(adcDry - adcWet)
	p := 100 * float64(adcDry-raw) / span
	return int(math.Round(math.Min(100, math.Max(0, p))))
}

// realRawReader reads the raw 12-bit ADC value [0-4095] of the channel
// sampleCount times and returns the rounded average.
func realRawReader(dev *common.I2CDevice, pin int) rawFunc {
	return func() (int, bool) {
		total := 0
		for i := 0; i < sampleCount; i++ {
			data, err := dev.ReadRegs(uint8(hatADCBase+pin), 2)
			if err != nil {
				fmt.Println("Sensor read failed:", err)
				return 0, false
			}
			total += int(data[0]) | int(data[1])<<8 // SMBus words are little-endian
			time.Sleep(2 * time.Millisecond)
		}
		return int(math.Round(float64(total) / sampleCount)), true
	}
}

// emulatedRawReader: a watering cycle — the moisture slowly dries from
// ~85 % down to ~25 % and jumps back up, on a few-minute loop for easy
// demoing. Returns raw values against the given calibration points.
func emulatedRawReader(adcDry, adcWet int) rawFunc {
	return func() (int, bool) {
		t := float64(time.Now().UnixNano()) / 1e9
		cycle := math.Mod(t, 300) / 300 // 0 -> 1 over five minutes
		pct := 85 - 60*cycle + 2*math.Sin(t/3)
		raw := float64(adcDry) - float64(adcDry-adcWet)*pct/100
		return int(math.Round(raw)), true
	}
}

func main() {
	cfg := Config{HatType: "grove", I2CBus: 1, AdcDry: 2600, AdcWet: 1100, Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	adcDry, adcWet := cfg.AdcDry, cfg.AdcWet
	var readRaw rawFunc
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating MOISTURE readings without hardware")
		// Like the Python node, the emulation ignores the calibration
		// config and uses the 12-bit defaults.
		adcDry, adcWet = 2600, 1100
		readRaw = emulatedRawReader(adcDry, adcWet)
	} else {
		if cfg.HatType != "grove" {
			fmt.Printf("Error: hat_type %q is not supported in the Go port (only \"grove\"; use the Python node for Arduino-based hats)\n", cfg.HatType)
			os.Exit(1)
		}
		if cfg.Pin < 0 || cfg.Pin > 7 {
			fmt.Printf("Error: invalid channel %d - valid range [0,7]\n", cfg.Pin)
			os.Exit(1)
		}
		if adcDry == adcWet {
			fmt.Println("Error: adc_dry and adc_wet must differ (calibrate!)")
			os.Exit(1)
		}
		fmt.Printf("Initializing moisture probe on grove hat, channel %d (dry=%d, wet=%d)...\n", cfg.Pin, adcDry, adcWet)
		dev, err := common.OpenI2C(cfg.I2CBus, hatI2CAddress)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		readRaw = realRawReader(dev, cfg.Pin)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	read := func() map[string]any {
		raw, ok := readRaw()
		if !ok {
			return nil
		}
		// The raw reading and its full scale help calibrate adc_dry/adc_wet.
		return map[string]any{
			"moisture": percent(raw, adcDry, adcWet),
			"adc":      raw,
			"adcMax":   hatADCMax,
		}
	}

	go func() {
		// Short-key reading for the discovery reply; identity-only on failure.
		err := common.RunDiscoveryListener("MOISTURE", cfg.Hostname, common.HTTPSPort,
			func() map[string]any {
				if raw, ok := readRaw(); ok {
					return map[string]any{"moist": percent(raw, adcDry, adcWet)}
				}
				return map[string]any{}
			})
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("MOISTURE", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
