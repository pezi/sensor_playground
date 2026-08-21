// Sensor Playground Display Node — SSD1306 128x64 OLED (Go)
//
// Implements the *display* variant of the Sensor Playground Sensor
// Interface on single-board computers (Raspberry Pi & co.) with an SSD1306
// I2C OLED. Unlike sensor nodes this node consumes data: the app pushes
// one JSON command per action over the WebSocket and the node draws it on
// the panel.
//
//	{"id": 1, "image": "<base64>"}   show a bitmap (1024 bytes, see below)
//	{"id": 2, "clear": true}         blank the display
//
// The node acknowledges each applied command with the matching id, or
// returns an error ACK when validation or the display write fails.
//
// Bitmap format (matches the dart_periphery SSD1306 example): 128x64
// pixels, 1 bit per pixel, packed horizontally row by row — 16 bytes per
// row, MSB of each byte is the leftmost pixel (the
// https://javl.github.io/image2cpp/ "horizontal" byte orientation). The
// node transposes this into the SSD1306 native page format before writing
// it over I2C.
//
//   - WebSocket server (ws://) on port 9132, X-Api-Key checked on the
//     handshake + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true to run without any hardware at all.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_ssd1306
package main

import (
	"fmt"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"

	"sensorplayground/common"
)

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	I2CBus     int    `json:"i2c_bus"`
	I2CAddress string `json:"i2c_address"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

// parseI2CAddress accepts the hex string the config carries ("0x3C"), the
// same form the Python node parses with int(str, 16).
func parseI2CAddress(text string) (uint8, error) {
	value, err := strconv.ParseUint(strings.TrimPrefix(strings.ToLower(text), "0x"), 16, 8)
	if err != nil {
		return 0, fmt.Errorf("i2c_address %q is not a hex address like \"0x3C\"", text)
	}
	return uint8(value), nil
}

func main() {
	cfg := Config{I2CBus: 1, I2CAddress: "0x3C", Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var display Display
	if cfg.Emulation {
		fmt.Println("Emulation mode: reporting frames without hardware")
		display = &EmulatedDisplay{}
	} else {
		address, err := parseI2CAddress(cfg.I2CAddress)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		fmt.Println("Initializing SSD1306 display...")
		panel, err := NewSSD1306(cfg.I2CBus, address)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		display = panel
	}

	// Blank the panel rather than leaving a stale image burning.
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-signals
		display.Clear()
		fmt.Println("Stopped.")
		os.Exit(0)
	}()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServerReplying(cfg.APIKey, nil,
		func(message []byte) any { return handleJSONCommand(display, message) },
	)

	go func() {
		err := common.RunDiscoveryListener("SSD1306", cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
