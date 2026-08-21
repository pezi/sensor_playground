// Sensor Playground Sensor Node — Grove NFC Tag (Go)
//
// Implements the *push* variant of the Sensor Playground Sensor Interface
// on single-board computers (Raspberry Pi & co.) with a Grove NFC Tag — a
// passive dual-interface EEPROM (ST M24LR64E-R, 8 KB). A phone or NFC
// writer stores an NDEF message over the ISO 15693 RF interface; this node
// reads the same memory over I2C, parses the first NDEF record and pushes
// one JSON message whenever the content changes:
//
//	{"kind": "text", "value": "Hello"}
//	{"kind": "uri",  "value": "https://seeed.cc"}
//	{"kind": "data", "value": "DEADBEEF"}      (hex, truncated)
//	{"kind": "empty"}
//
// Unlike the pure event sensors the tag holds state, so the current
// content is also sent to every client right after it connects.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to cycle through generated contents
// without the tag hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_nfctag
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"

	"sensorplayground/common"
)

// Seconds between EEPROM polls; an RF write shows up on the next poll.
const pollInterval = 1 * time.Second

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	I2CBus     int    `json:"i2c_bus"`
	I2CAddress string `json:"i2c_address"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

// -- Emulation ----------------------------------------------------------------

// EmulatedNfcTag cycles through generated tag contents without hardware.
type EmulatedNfcTag struct {
	index  int
	nextAt time.Time
}

var emulatedContents = []Content{
	{Kind: "text", Value: "Hello from Sensor Playground"},
	{Kind: "uri", Value: "https://wiki.seeedstudio.com/Grove_NFC_Tag/"},
	{Kind: "empty"},
}

func NewEmulatedNfcTag() *EmulatedNfcTag {
	return &EmulatedNfcTag{nextAt: time.Now().Add(15 * time.Second)}
}

// ReadContent returns the current fake content, advancing every 15 seconds.
func (t *EmulatedNfcTag) ReadContent() (Content, error) {
	if !time.Now().Before(t.nextAt) {
		t.index = (t.index + 1) % len(emulatedContents)
		t.nextAt = time.Now().Add(15 * time.Second)
	}
	return emulatedContents[t.index], nil
}

func (t *EmulatedNfcTag) Close() {}

// -- Content state ------------------------------------------------------------

// State holds the last published content: the tag holds state, so a client
// that just connected gets the current content instead of waiting for the
// next RF write.
type State struct {
	mu      sync.Mutex
	content *Content
}

// Update stores content and reports whether it differs from the last one.
func (s *State) Update(content Content) bool {
	s.mu.Lock()
	defer s.mu.Unlock()
	if s.content != nil && *s.content == content {
		return false
	}
	s.content = &content
	return true
}

// Current returns the last published content, or nil before the first read.
func (s *State) Current() *Content {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.content
}

// contentLoop polls the tag and pushes the content whenever it changes.
func contentLoop(tag Tag, state *State, publish func(payload any)) {
	for {
		content, err := tag.ReadContent()
		if err != nil {
			fmt.Printf("Tag read failed: %v\n", err)
		} else if state.Update(content) {
			message, _ := json.Marshal(content.Payload())
			fmt.Printf("Content: %s\n", message)
			publish(content.Payload())
		}
		time.Sleep(pollInterval)
	}
}

// -- Main --------------------------------------------------------------------

func main() {
	cfg := Config{I2CBus: 1, I2CAddress: "0x53", Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var tag Tag
	if cfg.Emulation {
		fmt.Println("Emulation mode: cycling NFC tag contents without hardware")
		tag = NewEmulatedNfcTag()
	} else {
		addr, err := strconv.ParseUint(strings.TrimPrefix(cfg.I2CAddress, "0x"), 16, 8)
		if err != nil {
			fmt.Printf("Error: config.json: invalid i2c_address %q\n", cfg.I2CAddress)
			os.Exit(1)
		}
		fmt.Printf("Opening M24LR64E on i2c bus %d...\n", cfg.I2CBus)
		tag, err = OpenM24lr64(cfg.I2CBus, uint8(addr))
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
	}
	defer tag.Close()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	state := &State{}
	server := common.NewWsServer(cfg.APIKey,
		func(send func(payload any) error) {
			if content := state.Current(); content != nil {
				send(content.Payload())
			}
		},
		nil, // push node: incoming messages are ignored
	)

	go func() {
		err := common.RunDiscoveryListener("NFCTAG", cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go contentLoop(tag, state, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
