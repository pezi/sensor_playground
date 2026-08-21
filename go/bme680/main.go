// Sensor Playground Sensor Node — BME680 (Go)
//
// Implements the Sensor Playground Sensor Interface on single-board
// computers (Raspberry Pi & co.) with a BME680 I2C sensor (temperature,
// humidity, pressure, IAQ).
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
//	./sensor_node_bme680
package main

import (
	"fmt"
	"os"

	"sensorplayground/common"
)

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	I2CBus    int    `json:"i2c_bus"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
	SSLCert   string `json:"ssl_cert"`
	SSLKey    string `json:"ssl_key"`
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

	var sensor Sensor
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating BME680 readings without hardware")
		sensor = NewEmulatedBME680Sensor()
	} else {
		fmt.Printf("Initializing BME680 sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		s, err := NewBME680Sensor(cfg.I2CBus)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		sensor = s
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	go func() {
		err := common.RunDiscoveryListener(sensor.Name(), cfg.Hostname, common.HTTPSPort,
			func() map[string]any { return ReadDiscovery(sensor) })
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	err := common.RunRESTServer(sensor.Name(), cfg.APIKey, cfg.Hostname,
		func() map[string]any { return ReadFull(sensor) }, cfg.SSLCert, cfg.SSLKey)
	if err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
