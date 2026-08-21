// Sensor Playground EEPROM Node — AT24C128 (Go)
//
// Implements the *actuator* variant of the Sensor Playground Sensor
// Interface on single-board computers (Raspberry Pi & co.) with an
// AT24C128 serial EEPROM (128 Kbit / 16 KB, I2C address 0x50). The app
// stores a short text on the chip and reads it back at any time; the text
// survives power cycles of both ends.
//
// Like the LED the node is the single source of truth: after a write it
// reads the chip back and reports the *stored* text, so a failed write
// cannot leave the app showing a text the chip never held.
//
//	app -> node   {"write": "Hello"}   store the text on the chip
//	              {"read": true}       re-read the chip and push
//	node -> app   {"text": "Hello"}    stored text (on connect and after
//	                                   every write/read, read from the chip)
//
// EEPROM layout (offset 0): magic 'S' 'P', u16 big-endian text length
// (max 512 bytes), then the UTF-8 text. A chip without the magic (e.g.
// factory-fresh, all 0xFF) reads as an empty text.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to run without the chip (the text
// then lives in memory only).
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_at24c128
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"sensorplayground/common"
)

// Seconds between pending-publication polls (commands only mark the state
// dirty; this loop is what puts it on the wire).
const pollInterval = 50 * time.Millisecond

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	SensorName string `json:"sensor_name"`
	I2CBus     int    `json:"i2c_bus"`
	I2CAddress string `json:"i2c_address"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

// -- Stored-text state -------------------------------------------------------

// EepromController owns the stored text — the single source of truth this
// node publishes. Every command marks the state as pending publication,
// even one that does not change it: the published text is always a fresh
// read-back, so the app renders what the chip actually holds.
type EepromController struct {
	mu      sync.Mutex
	eeprom  Eeprom
	text    string
	pending bool
}

func NewEepromController(eeprom Eeprom) *EepromController {
	controller := &EepromController{eeprom: eeprom}
	controller.Read() // publish the initial text as soon as we serve
	return controller
}

// Write stores [text] on the chip and re-reads it.
func (c *EepromController) Write(text []byte) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(text) > textMaxBytes {
		fmt.Printf("Rejecting write of %d bytes (max %d)\n", len(text), textMaxBytes)
	} else if err := c.eeprom.WriteText(text); err != nil {
		fmt.Println("EEPROM write failed:", err)
	}
	c.reread()
}

// Read re-reads the chip and marks the text for publication.
func (c *EepromController) Read() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.reread()
}

// reread refreshes the text from the chip; the caller holds the lock. A
// failed read keeps the last known text rather than blanking the app.
func (c *EepromController) reread() {
	text, err := c.eeprom.ReadText()
	if err != nil {
		fmt.Println("EEPROM read failed:", err)
	} else {
		c.text = text
	}
	c.pending = true
}

func (c *EepromController) Text() string {
	c.mu.Lock()
	defer c.mu.Unlock()
	return c.text
}

// TakePending returns true once after each command, clearing the flag.
func (c *EepromController) TakePending() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	pending := c.pending
	c.pending = false
	return pending
}

func (c *EepromController) Close() { c.eeprom.Close() }

// -- Commands ----------------------------------------------------------------

// handleJSONCommand executes one JSON command pushed by the app over the
// WebSocket.
func handleJSONCommand(controller *EepromController, message []byte) {
	var command map[string]any
	if err := json.Unmarshal(message, &command); err != nil {
		fmt.Println("Ignoring malformed command")
		return
	}
	if command["read"] == true {
		fmt.Println("Command: read")
		controller.Read()
		return
	}
	text, ok := command["write"].(string)
	if !ok {
		fmt.Println("Ignoring command without a string 'write'")
		return
	}
	fmt.Printf("Command: write %d characters\n", utf8.RuneCountInString(text))
	controller.Write([]byte(text))
}

// -- Text loop ---------------------------------------------------------------

// textLoop publishes the stored text whenever a command marked it pending.
// Command handlers run inside the WebSocket server's dispatch, so they only
// mutate the controller and this loop puts the result on the wire.
func textLoop(controller *EepromController, publish func(payload any)) {
	for {
		if controller.TakePending() {
			text := controller.Text()
			fmt.Printf("text: %q\n", text)
			publish(map[string]any{"text": text})
		}
		time.Sleep(pollInterval)
	}
}

// -- Main --------------------------------------------------------------------

func main() {
	cfg := Config{SensorName: "AT24C128", I2CBus: 1, I2CAddress: "0x50", Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var eeprom Eeprom
	if cfg.Emulation {
		fmt.Println("Emulation mode: storing the text in memory without hardware")
		eeprom = NewEmulatedEeprom()
	} else {
		addr, err := strconv.ParseUint(strings.TrimPrefix(cfg.I2CAddress, "0x"), 16, 8)
		if err != nil {
			fmt.Printf("Error: config.json: invalid i2c_address %q\n", cfg.I2CAddress)
			os.Exit(1)
		}
		fmt.Printf("Opening AT24C128 on i2c bus %d...\n", cfg.I2CBus)
		eeprom, err = OpenAt24c128(cfg.I2CBus, uint8(addr))
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
	}

	controller := NewEepromController(eeprom)
	defer controller.Close()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey,
		func(send func(payload any) error) {
			// The chip holds state: a client that just connected gets the
			// stored text instead of an empty field.
			send(map[string]any{"text": controller.Text()})
		},
		func(message []byte) { handleJSONCommand(controller, message) },
	)

	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go textLoop(controller, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
