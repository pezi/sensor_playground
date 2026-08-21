// Sensor Playground Relay Node — Grove SPDT Relay, 1 / 2 / 4 channels (Go)
//
// The app switches individual channels or the complete board, and the node
// reports the resulting state of every channel:
//
//	app -> node  {"ch":0,"on":true}      switch channel 0
//	             {"ch":0,"toggle":true}  toggle channel 0
//	             {"all":false}           switch every channel off
//	node -> app  {"channels":2,"relay":[true,false]}
//
// GPIO character-device lines drive the 1-/2-channel modules. The Grove
// 4-channel module is driven directly over I2C with command 0x10 + bitmask.
// WebSocket commands/state use port 9132 and UDP discovery uses 9133.
// BLE is not supported by the Go common transport and falls back to Wi-Fi.
package main

import (
	"encoding/json"
	"fmt"
	"io"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"
	"time"

	"sensorplayground/common"
)

const publishInterval = 20 * time.Millisecond

type Config struct {
	APIKey         string `json:"api_key"`
	Hostname       string `json:"hostname"`
	SensorName     string `json:"sensor_name"`
	Interface      string `json:"interface"`
	GPIOChip       string `json:"gpio_chip"`
	RelayPins      []int  `json:"relay_pins"`
	RelayActiveLow bool   `json:"relay_active_low"`
	Channels       int    `json:"channels"`
	I2CAddress     string `json:"i2c_address"`
	I2CBus         int    `json:"i2c_bus"`
	Transport      string `json:"transport"`
	Emulation      bool   `json:"emulation"`
}

func configuredChannelCount(cfg Config) (int, error) {
	if cfg.Interface == "i2c" {
		if cfg.Channels < 1 || cfg.Channels > 8 {
			return 0, fmt.Errorf("config.json: channels must be 1..8")
		}
		return cfg.Channels, nil
	}
	if len(cfg.RelayPins) < 1 || len(cfg.RelayPins) > 8 {
		return 0, fmt.Errorf("config.json: relay_pins must list 1..8 GPIO lines")
	}
	return len(cfg.RelayPins), nil
}

func makeRelayBank(cfg Config) (RelayBank, error) {
	count, err := configuredChannelCount(cfg)
	if err != nil {
		return nil, err
	}
	if cfg.Emulation {
		return &emulatedRelayBank{count: count}, nil
	}
	switch cfg.Interface {
	case "gpio":
		return newGPIOBank(cfg.GPIOChip, cfg.RelayPins, cfg.RelayActiveLow)
	case "i2c":
		address, err := parseI2CAddress(cfg.I2CAddress)
		if err != nil {
			return nil, err
		}
		return newI2CBank(cfg.I2CBus, address, cfg.Channels)
	default:
		return nil, fmt.Errorf("interface %q is not supported (use \"gpio\" or \"i2c\"; Arduino-based hats require the Python port)", cfg.Interface)
	}
}

func parseI2CAddress(text string) (uint8, error) {
	address, err := strconv.ParseUint(strings.TrimPrefix(strings.TrimPrefix(text, "0x"), "0X"), 16, 8)
	if err != nil {
		return 0, fmt.Errorf("config.json: invalid i2c_address %q", text)
	}
	if address > 0x7f {
		return 0, fmt.Errorf("config.json: i2c_address %q is outside 0x00..0x7f", text)
	}
	return uint8(address), nil
}

func channelFrom(value any) (int, bool) {
	number, ok := value.(json.Number)
	if !ok {
		return 0, false
	}
	channel, err := strconv.Atoi(number.String())
	return channel, err == nil
}

func handleJSONCommand(relay *RelayController, message []byte) {
	decoder := json.NewDecoder(strings.NewReader(string(message)))
	decoder.UseNumber()
	var command map[string]any
	if err := decoder.Decode(&command); err != nil {
		fmt.Println("Ignoring malformed command")
		return
	}
	if err := decoder.Decode(&struct{}{}); err != io.EOF {
		fmt.Println("Ignoring malformed command")
		return
	}
	if on, ok := command["all"].(bool); ok {
		fmt.Printf("Command: all %s\n", onOff(on))
		if err := relay.SetAll(on); err != nil {
			fmt.Println("Relay write failed:", err)
		}
		return
	}
	channel, ok := channelFrom(command["ch"])
	if !ok {
		fmt.Println("Ignoring command without an integer channel")
		return
	}
	if command["toggle"] == true {
		fmt.Printf("Command: toggle channel %d\n", channel)
		if err := relay.Toggle(channel); err != nil {
			fmt.Println("Ignoring command:", err)
		}
		return
	}
	on, ok := command["on"].(bool)
	if !ok {
		fmt.Println("Ignoring command without a boolean 'on'")
		return
	}
	fmt.Printf("Command: channel %d %s\n", channel, onOff(on))
	if err := relay.Set(channel, on); err != nil {
		fmt.Println("Ignoring command:", err)
	}
}

func onOff(on bool) string {
	if on {
		return "on"
	}
	return "off"
}

func stateLoop(relay *RelayController, publish func(any)) {
	ticker := time.NewTicker(publishInterval)
	defer ticker.Stop()
	for range ticker.C {
		if payload, pending := relay.TakePending(); pending {
			fmt.Printf("relay: %v\n", payload["relay"])
			publish(payload)
		}
	}
}

func main() {
	cfg := Config{
		SensorName: "RELAY", Interface: "gpio", GPIOChip: "/dev/gpiochip0",
		RelayPins: []int{5, 6}, Channels: 4, I2CAddress: "0x11", I2CBus: 1,
		Transport: "wifi",
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

	if cfg.Emulation {
		fmt.Println("Emulation mode: switching virtual relays without hardware")
	} else {
		fmt.Println("Initializing relay node...")
	}
	bank, err := makeRelayBank(cfg)
	if err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	relay, err := NewRelayController(bank)
	if err != nil {
		bank.Close()
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	fmt.Printf("Relay board: %d channel(s)\n", relay.Count())

	// Leave every contact open rather than stuck closed after the node exits.
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-signals
		if err := relay.SetAll(false); err != nil {
			fmt.Println("Relay shutdown failed:", err)
		}
		relay.Close()
		fmt.Println("Stopped.")
		os.Exit(0)
	}()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(
		cfg.APIKey,
		func(send func(any) error) { _ = send(relay.Payload()) },
		func(message []byte) { handleJSONCommand(relay, message) },
	)
	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.WSPort, func() map[string]any {
			return map[string]any{"channels": relay.Count()}
		})
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go stateLoop(relay, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		_ = relay.SetAll(false)
		_ = relay.Close()
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
