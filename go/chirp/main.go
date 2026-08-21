// Sensor Playground Sensor Node — Chirp I2C Soil Moisture Sensor (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers with a Catnip Electronics I2C Soil Moisture Sensor (the
// "Chirp" sensor, default I2C address 0x20). It reports the soil
// moisture as a percentage (JSON key `moisture`, mapped linearly between
// the two capacitance calibration points in config.json), the soil
// temperature (`temperature`) and the ambient light level (`light`, raw
// brightness counts, higher = brighter), alongside the raw capacitance
// (`cap`) for calibrating.
//
// The chip measures light by timing a phototransistor discharge, which
// takes up to three seconds — so the light value is harvested from a
// measurement started on an earlier read, and the `light` key is absent
// until the first one completes (a few seconds after start).
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
//	./sensor_node_chirp
package main

import (
	"fmt"
	"math"
	"os"
	"time"

	"sensorplayground/common"
)

func round1(x float64) float64 { return math.Round(x*10) / 10 }

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	I2CBus    int    `json:"i2c_bus"`
	Address   int    `json:"address"`
	CapDry    int    `json:"cap_dry"`
	CapWet    int    `json:"cap_wet"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
	SSLCert   string `json:"ssl_cert"`
	SSLKey    string `json:"ssl_key"`
}

// read returns the full-key REST payload, or nil on a failed read.
type readFunc func() map[string]any

func realReader(s *Chirp, capDry, capWet int) readFunc {
	return func() map[string]any {
		r, err := s.Read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		reading := map[string]any{
			"moisture":    moisturePercent(r.Capacitance, capDry, capWet),
			"temperature": r.Temperature,
			// The raw capacitance helps calibrate cap_dry/cap_wet.
			"cap": r.Capacitance,
		}
		if r.HasLight {
			reading["light"] = r.Light
		}
		return reading
	}
}

// emulatedReader generates plausible readings without hardware: a
// watering cycle for the moisture, a steady room temperature and a slow
// day/night curve for the light. Like the Python node, the emulation
// ignores the configured calibration points and uses the defaults.
func emulatedReader() readFunc {
	const capDry, capWet = 290, 520
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		cycle := math.Mod(t, 300) / 300 // rewatered every five minutes
		pct := 85 - 60*cycle + 2*math.Sin(t/3)
		capacitance := int(math.Round(capDry + (capWet-capDry)*pct/100))
		rawTemp := uint16(int16(math.Round(10 * (21.5 + 1.5*math.Sin(t/60)))))
		rawLight := uint16(math.Round(20000 + 15000*math.Sin(t/120)))
		return map[string]any{
			"moisture":    moisturePercent(capacitance, capDry, capWet),
			"temperature": decodeTemperature(rawTemp),
			"cap":         capacitance,
			// The emulated measurement finishes instantly, so unlike the
			// real sensor the key is present from the first read.
			"light": lightCounts(rawLight),
		}
	}
}

// discoveryFor builds the short-key reading for the discovery reply;
// identity-only on failure.
func discoveryFor(read readFunc) func() map[string]any {
	return func() map[string]any {
		full := read()
		if full == nil {
			return map[string]any{}
		}
		short := map[string]any{
			"moist": full["moisture"],
			"temp":  full["temperature"],
		}
		if light, ok := full["light"]; ok {
			short["light"] = light
		}
		return short
	}
}

func main() {
	cfg := Config{I2CBus: 1, Address: 0x20, CapDry: 290, CapWet: 520, Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
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
		fmt.Println("Emulation mode: generating CHIRP readings without hardware")
		read = emulatedReader()
	} else {
		if cfg.CapDry == cfg.CapWet {
			fmt.Println("Error: cap_dry and cap_wet must differ (calibrate!)")
			os.Exit(1)
		}
		fmt.Printf("Initializing Chirp sensor on I2C bus %d (dry=%d, wet=%d)...\n",
			cfg.I2CBus, cfg.CapDry, cfg.CapWet)
		sensor, err := NewChirp(cfg.I2CBus, cfg.Address)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = realReader(sensor, cfg.CapDry, cfg.CapWet)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener("CHIRP", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("CHIRP", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
