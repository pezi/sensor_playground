// Sensor Playground Sensor Node — Grove 125KHz RFID Reader (Go)
//
// Implements the *push* variant of the Sensor Playground Sensor Interface
// on single-board computers (Raspberry Pi & co.) with a Grove 125KHz RFID
// Reader. The node reads RDM630-style frames from a 9600-baud UART and
// pushes one JSON message ({"tag": "0F0024ADAB"}) per scanned EM4100 tag.
//
// Frame format (reader TX, jumper on UART mode — not Wiegand):
//
//	STX 0x02 | 10 ASCII-hex data chars | 2 ASCII-hex checksum chars | ETX 0x03
//
// The checksum byte is the XOR of the five data bytes. The reader repeats
// the frame while a tag is held near the antenna, so the node suppresses
// repeats of the same tag for repeatSuppress.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to generate plausible scans without
// the reader hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_rfid
package main

import (
	"fmt"
	"math/rand"
	"os"
	"time"

	"sensorplayground/common"
)

const (
	// Suppress repeats of the same tag while it is held near the antenna.
	repeatSuppress = 2 * time.Second
)

func uniform(lo, hi float64) float64 { return lo + rand.Float64()*(hi-lo) }

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	SerialPort string `json:"serial_port"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

// -- Readers -----------------------------------------------------------------

// runSerialReader drains the UART continuously and sends each validated
// tag on the channel — the reader is transmit-only, so frames simply
// arrive whenever a tag is presented.
func runSerialReader(port *SerialPort, tags chan<- string) {
	parser := &FrameParser{}
	buf := make([]byte, 64)
	for {
		n, err := port.Read(buf)
		if err != nil {
			fmt.Println("Serial read failed:", err)
			return
		}
		for _, b := range buf[:n] {
			if tag, ok := parser.Feed(b); ok {
				tags <- tag
			}
		}
	}
}

// runEmulatedReader generates plausible RFID scans without hardware: one
// tag from a small fixed pool every four to eight seconds.
func runEmulatedReader(tags chan<- string) {
	pool := []string{"0F0024ADAB", "0A0031B2C4", "03004F19AA", "1000C0FFEE"}
	for {
		time.Sleep(time.Duration(uniform(4.0, 8.0) * float64(time.Second)))
		tags <- pool[rand.Intn(len(pool))]
	}
}

// -- Tag loop ----------------------------------------------------------------

// tagLoop pushes each scanned tag, suppressing repeats of the same tag
// while it is held near the antenna.
func tagLoop(tags <-chan string, publish func(payload any)) {
	var lastTag string
	var lastAt time.Time
	for tag := range tags {
		if tag == lastTag && time.Since(lastAt) < repeatSuppress {
			continue
		}
		fmt.Println("Tag:", tag)
		publish(map[string]any{"tag": tag})
		lastTag = tag
		lastAt = time.Now()
	}
}

// -- Main --------------------------------------------------------------------

func main() {
	cfg := Config{SerialPort: "/dev/serial0", Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	tags := make(chan string, 8)
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating RFID scans without hardware")
		go runEmulatedReader(tags)
	} else {
		fmt.Printf("Opening RFID reader on %s...\n", cfg.SerialPort)
		port, err := OpenSerial(cfg.SerialPort)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		go runSerialReader(port, tags)
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey, nil, nil)

	go func() {
		err := common.RunDiscoveryListener("RFID", cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go tagLoop(tags, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
