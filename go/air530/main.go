// Sensor Playground Sensor Node — Air530 GPS (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a Grove GPS (Air530) module. Like
// the CozIR the Air530 is not an I2C device: it continuously streams
// NMEA-0183 sentences over a 9600-baud UART (serial). The node reads one
// burst per request and parses the fix (port of the dart_periphery
// NmeaParser, see serial_air530.dart):
//
//   - GGA sentences (preferred): latitude, longitude, MSL altitude,
//     satellites in use
//
//   - GLL sentences (fallback): latitude, longitude only
//
//   - Sentences with bad checksums are skipped
//
//   - HTTPS REST API on port 9132 + UDP discovery on port 9133
//
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     Wi-Fi with a warning (use the Python or Rust node for BLE).
//
// Until the module has a position fix the read yields an empty map — not
// nil — so the REST API answers 200 with a metadata-only body instead of
// the shared transport's 503 (the Python node's allow_empty behavior; the
// app then shows its "waiting for satellite fix" screen).
//
// Set "emulation": true in config.json to generate plausible readings
// without the sensor hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_air530
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"sync"
	"time"

	"sensorplayground/common"
)

func round1(x float64) float64 { return math.Round(x*10) / 10 }

func uniform(lo, hi float64) float64 { return lo + rand.Float64()*(hi-lo) }

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	SerialPort string `json:"serial_port"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
	SSLCert    string `json:"ssl_cert"`
	SSLKey     string `json:"ssl_key"`
}

type readFunc func() map[string]any

// realReader reads one ~1 Hz NMEA burst per request; partial first lines
// fail the checksum and are skipped by the parser. Without a fix it
// returns an empty map (200 metadata-only), never nil (503) — a GPS
// still warming up is not an error.
func realReader(port *SerialPort) readFunc {
	var mu sync.Mutex
	return func() map[string]any {
		mu.Lock()
		port.FlushInput()
		raw := port.ReadBlock(512)
		mu.Unlock()

		fix := parseNMEA(raw)
		if fix == nil {
			return map[string]any{}
		}
		return fix
	}
}

// emulatedReader: a receiver at St. Stephen's Cathedral in Vienna
// (48.2085 N, 16.3730 E) — the position performs a tiny random walk
// around the base coordinate, the altitude drifts slowly around 171 m MSL
// and the satellite count varies between 4 and 12.
func emulatedReader() readFunc {
	lat := 48.2085
	lon := 16.3730
	return func() map[string]any {
		t := float64(time.Now().UnixNano()) / 1e9
		lat += uniform(-0.0001, 0.0001)
		lon += uniform(-0.0001, 0.0001)
		return map[string]any{
			"latitude":   round6(lat),
			"longitude":  round6(lon),
			"altitude":   round1(171.0 + 5.0*math.Sin(t/120.0)),
			"satellites": 4 + rand.Intn(9),
		}
	}
}

func discoveryFor(read readFunc) func() map[string]any {
	return func() map[string]any {
		full := read()
		short := map[string]any{}
		if full == nil || full["latitude"] == nil {
			return short
		}
		short["lat"] = full["latitude"]
		short["lon"] = full["longitude"]
		if alt, ok := full["altitude"]; ok {
			short["alt"] = alt
		}
		if sats, ok := full["satellites"]; ok {
			short["sats"] = sats
		}
		return short
	}
}

func main() {
	cfg := Config{SerialPort: "/dev/serial0", Transport: "wifi", SSLCert: "cert.pem", SSLKey: "key.pem"}
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
		fmt.Println("Emulation mode: generating Air530 readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing Air530 GPS on %s...\n", cfg.SerialPort)
		port, err := OpenSerial(cfg.SerialPort)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		read = realReader(port)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener("AIR530", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("AIR530", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
