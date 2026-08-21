// Sensor Playground Sensor Node — CozIR CO2 Sensor (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a CozIR-A sensor (temperature,
// humidity, CO2). Unlike the other environment sensors the CozIR is not an
// I2C device: it talks a simple ASCII command protocol over a 9600-baud
// UART (serial).
//
// Protocol (see the Python node and dart_periphery's serial_cozir.dart):
//
//		M 4164\r\n   select humidity, temperature and CO2 output fields
//		K 2\r\n      polling mode
//		Q\r\n        request one measurement:
//		             "H 00495 T 01234 Z 06399" -> 49.5 %RH, 23.4 degC, 639.9 ppm
//
//	  - HTTPS REST API on port 9132 + UDP discovery on port 9133
//	  - BLE is not supported in this port; "transport": "ble" falls back to
//	    Wi-Fi with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to generate plausible readings
// without the sensor hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_cozir
package main

import (
	"fmt"
	"math"
	"math/rand"
	"os"
	"regexp"
	"strconv"
	"sync"
	"time"

	"sensorplayground/common"
)

// One measurement line: "H 00495 T 01234 Z 06399" (leading space may occur).
var measurementPattern = regexp.MustCompile(`H\s+(\d+)\s+T\s+(\d+)\s+Z\s+(\d+)`)

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

// realReader polls the CozIR: one Q command, one measurement line.
func realReader(port *SerialPort) readFunc {
	var mu sync.Mutex
	return func() map[string]any {
		mu.Lock()
		port.FlushInput()
		if err := port.Write([]byte("Q\r\n")); err != nil {
			mu.Unlock()
			fmt.Println("Sensor read failed:", err)
			return nil
		}
		raw := port.ReadLine(64)
		mu.Unlock()

		match := measurementPattern.FindStringSubmatch(raw)
		if match == nil {
			return nil
		}
		hum, _ := strconv.Atoi(match[1])
		temp, _ := strconv.Atoi(match[2])
		co2, _ := strconv.Atoi(match[3])
		return map[string]any{
			"temperature": round1(float64(temp-1000) / 10.0),
			"humidity":    round1(float64(hum) / 10.0),
			"co2":         round1(float64(co2) / 10.0),
		}
	}
}

// emulatedReader: a quiet indoor room — slow sines for temperature and
// humidity, a bounded random walk between 400 and 1500 ppm for the CO2.
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
		fmt.Println("Emulation mode: generating CozIR readings without hardware")
		read = emulatedReader()
	} else {
		fmt.Printf("Initializing CozIR sensor on %s...\n", cfg.SerialPort)
		port, err := OpenSerial(cfg.SerialPort)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		// Select the humidity, temperature and CO2 output fields, then
		// switch to polling mode (one measurement per Q command).
		port.Write([]byte("M 4164\r\n"))
		port.Write([]byte("K 2\r\n"))
		port.FlushInput()
		read = realReader(port)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener("COZIR", cfg.Hostname, common.HTTPSPort, discoveryFor(read))
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := common.RunRESTServer("COZIR", cfg.APIKey, cfg.Hostname, read, cfg.SSLCert, cfg.SSLKey); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
