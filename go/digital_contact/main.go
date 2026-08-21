// Sensor Playground Sensor Node — Digital Contact Sensor (Go, push)
//
// One generic *push* node for the simple two-state Grove/BakeBit digital
// sensors that react to an event:
//
//	BUTTON     — Grove/BakeBit button      (pressed)
//	HALL       — Grove Hall sensor         (magnetic field present)
//	MAGSWITCH  — Grove magnetic switch     (reed switch closed)
//	PIR        — Grove PIR motion sensor   (motion detected)
//	VIBRATION  — Grove vibration sensor    (SW-420, vibration)
//	LINEFINDER — Grove Line Finder         (dark line under the sensor)
//
// The node polls the input (debounced) and pushes one JSON message whenever
// the state changes:
//
//	{"active": true}    sensor triggered
//	{"active": false}   sensor released
//
// The input is read from a GPIO character-device line ("interface": "gpio",
// /dev/gpiochipN — /sys/class/gpio is gone in Debian 13). The Arduino-based
// extension hats the Python node also supports ("interface": "hat") are not
// implemented in this port.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true in config.json to generate plausible readings
// without the sensor hardware.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_digital_contact
package main

import (
	"fmt"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"sensorplayground/common"
)

const (
	pollInterval = 20 * time.Millisecond // 50 Hz input poll
	debounce     = 30 * time.Millisecond // line must be stable this long before a change counts
)

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	SensorName string `json:"sensor_name"`
	ActiveLow  *bool  `json:"active_low"`
	Interface  string `json:"interface"`
	GpioChip   string `json:"gpio_chip"`
	Pin        *int   `json:"pin"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

// -- Inputs ------------------------------------------------------------------

type Input interface {
	ReadActive() bool
	Close() error
}

// gpioInput reads the digital input from a GPIO character-device line.
// The line is opened with the matching internal pull resistor and the
// kernel's active-low translation, so ReadActive is already "sensor
// triggered" — the same semantics as gpiozero's pull_up in the Python
// node.
type gpioInput struct{ line *common.GpioLine }

func (i *gpioInput) ReadActive() bool {
	active, err := i.line.Get()
	return err == nil && active
}
func (i *gpioInput) Close() error { return i.line.Close() }

// emulatedInput generates plausible digital contact readings without
// hardware: the contact toggles roughly every five seconds, as if someone
// slowly pressed and released a button.
type emulatedInput struct{}

func (emulatedInput) ReadActive() bool { return time.Now().Unix()/5%2 == 0 }
func (emulatedInput) Close() error     { return nil }

// -- State -------------------------------------------------------------------

// Latest published state, shared with newly connecting clients (nil until
// the first debounced reading).
var (
	stateMu     sync.Mutex
	stateActive *bool
)

func setState(active bool) {
	stateMu.Lock()
	defer stateMu.Unlock()
	stateActive = &active
}

func currentState() (active, known bool) {
	stateMu.Lock()
	defer stateMu.Unlock()
	if stateActive == nil {
		return false, false
	}
	return *stateActive, true
}

// -- Input loop --------------------------------------------------------------

// inputLoop polls the input (debounced) and pushes each state change.
func inputLoop(sensor Input, publish func(payload any)) {
	var stable, candidate *bool // last published state / pending state awaiting debounce
	var candidateSince time.Time

	for {
		active := sensor.ReadActive()
		now := time.Now()
		if candidate == nil || active != *candidate {
			v := active
			candidate = &v
			candidateSince = now
		} else if (stable == nil || active != *stable) && now.Sub(candidateSince) >= debounce {
			v := active
			stable = &v
			setState(active)
			fmt.Printf("active: %v\n", active)
			publish(map[string]any{"active": active})
		}
		time.Sleep(pollInterval)
	}
}

// -- Main --------------------------------------------------------------------

func main() {
	cfg := Config{SensorName: "BUTTON", Interface: "gpio", GpioChip: "/dev/gpiochip0", Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	activeLow := true
	if cfg.ActiveLow != nil {
		activeLow = *cfg.ActiveLow
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var sensor Input
	if cfg.Emulation {
		fmt.Printf("Emulation mode: generating %s readings without hardware\n", cfg.SensorName)
		sensor = emulatedInput{}
	} else {
		fmt.Printf("Initializing %s digital contact sensor...\n", cfg.SensorName)
		if cfg.Interface != "gpio" {
			fmt.Printf("Error: interface %q is not supported in the Go port (only \"gpio\"; use the Python node for Arduino-based hats)\n", cfg.Interface)
			os.Exit(1)
		}
		if cfg.Pin == nil {
			fmt.Println("Error: config.json: pin is required")
			os.Exit(1)
		}
		line, err := common.OpenInputLine(cfg.GpioChip, *cfg.Pin, activeLow)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		sensor = &gpioInput{line: line}
	}

	// Release the GPIO line on shutdown.
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-signals
		sensor.Close()
		fmt.Println("Stopped.")
		os.Exit(0)
	}()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	// Send the current state to a client right after it connects.
	server := common.NewWsServer(cfg.APIKey,
		func(send func(payload any) error) {
			if active, known := currentState(); known {
				send(map[string]any{"active": active})
			}
		},
		nil,
	)

	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go inputLoop(sensor, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
