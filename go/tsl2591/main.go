// Sensor Playground Sensor Node — TSL2591 (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a TSL2591 I2C sensor (visible light,
// infrared, illuminance).
//
// The chip has two photodiodes: channel 0 is broadband (visible + IR) and
// channel 1 is infrared only. Neither is "visible light" on its own — the
// difference of the two is. Lux is a third thing again, derived from both
// channels together with the gain and integration time.
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
//	./sensor_node_tsl2591
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
	APIKey        string `json:"api_key"`
	Hostname      string `json:"hostname"`
	I2CBus        int    `json:"i2c_bus"`
	Gain          string `json:"gain"`
	IntegrationMs int    `json:"integration_ms"`
	AutoGain      *bool  `json:"auto_gain"` // pointer: absent means true
	Transport     string `json:"transport"`
	Emulation     bool   `json:"emulation"`
	SSLCert       string `json:"ssl_cert"`
	SSLKey        string `json:"ssl_key"`
}

// read returns the full-key REST payload, or nil on a failed read. The lux
// key is absent when a channel saturated.
type readFunc func() map[string]any

func realReader(s *TSL2591) readFunc {
	return func() map[string]any {
		broadband, infrared, err := s.RawLuminosity()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		// Floored: in near-darkness noise can put IR marginally above the
		// broadband channel, and a negative amount of visible light is
		// meaningless on a chart.
		visible := 0
		if broadband > infrared {
			visible = int(broadband) - int(infrared)
		}
		reading := map[string]any{"visible": visible, "ir": int(infrared)}
		// A saturated channel makes the lux value wrong. The counts still
		// show the app that it is very bright, so omit only the lux rather
		// than publish a wrong value — the metric registry renders whatever
		// keys are present.
		if lux, ok := s.Lux(broadband, infrared); ok {
			reading["lux"] = round1(lux)
		}
		s.AutoGainStep(broadband)
		return reading
	}
}

// emulatedReader generates plausible readings without hardware: a lit room
// near a window — a few hundred lux drifting on a slow cycle, with the
// infrared channel holding the roughly one-third share of the broadband
// count that daylight and incandescent lamps produce (like the Python node).
func emulatedReader() readFunc {
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		broadband := math.Round(5400 + 1200*math.Sin(t/90.0) + uniform(-40, 40))
		infrared := math.Round(broadband*0.28 + uniform(-20, 20))
		return map[string]any{
			"visible": int(math.Max(0, broadband-infrared)),
			"ir":      int(infrared),
			"lux":     round1(310 + 70*math.Sin(t/90.0) + uniform(-2, 2)),
		}
	}
}

func discoveryFor(read readFunc) func() map[string]any {
	return func() map[string]any {
		full := read()
		if full == nil {
			return map[string]any{}
		}
		short := map[string]any{"vis": full["visible"], "ir": full["ir"]}
		if lux, present := full["lux"]; present {
			short["lux"] = lux
		}
		return short
	}
}

func main() {
	cfg := Config{
		I2CBus:        1,
		Gain:          "med",
		IntegrationMs: 300,
		Transport:     "wifi",
		SSLCert:       "cert.pem",
		SSLKey:        "key.pem",
	}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)
	autoGain := cfg.AutoGain == nil || *cfg.AutoGain

	var read readFunc
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating TSL2591 readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing TSL2591 sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		sensor, err := NewTSL2591(cfg.I2CBus, cfg.Gain, cfg.IntegrationMs, autoGain)
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
		err := common.RunDiscoveryListener("TSL2591", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("TSL2591", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
