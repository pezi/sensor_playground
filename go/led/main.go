// Sensor Playground LED Node — LED + optional push button (Go)
//
// Implements the *actuator* variant of the Sensor Playground Sensor
// Interface: the app sends a switch command and the node reports the
// resulting state back, because the LED can also be toggled by a push
// button wired to the node itself.
//
//	app -> node   {"led": true}     switch on
//	              {"led": false}    switch off
//	              {"toggle": true}  flip
//	node -> app   {"led": true|false}   current state (on connect and after
//	                                    every change, whatever caused it)
//
// The node owns the state; the app renders what the node last reported.
//
// The LED and button are driven from GPIO character-device lines
// ("interface": "gpio", /dev/gpiochipN — /sys/class/gpio is gone in
// Debian 13). The Arduino-based extension hats the Python node also
// supports ("interface": "hat") are not implemented in this port.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "button_pin": null for an LED-only node, and "emulation": true to
// run without any hardware at all.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_led
package main

import (
	"encoding/json"
	"fmt"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"sensorplayground/common"
)

const (
	pollInterval = 20 * time.Millisecond // 50 Hz button poll
	debounce     = 30 * time.Millisecond // stable this long before a press counts
)

type Config struct {
	APIKey          string `json:"api_key"`
	Hostname        string `json:"hostname"`
	SensorName      string `json:"sensor_name"`
	Interface       string `json:"interface"`
	GpioChip        string `json:"gpio_chip"`
	LedPin          *int   `json:"led_pin"`
	LedActiveLow    bool   `json:"led_active_low"`
	ButtonPin       *int   `json:"button_pin"`
	ButtonActiveLow *bool  `json:"button_active_low"`
	Transport       string `json:"transport"`
	Emulation       bool   `json:"emulation"`
}

// -- Outputs -----------------------------------------------------------------

type Output interface {
	Write(on bool) error
	Close() error
}

type gpioOutput struct{ line *common.GpioLine }

func (o *gpioOutput) Write(on bool) error { return o.line.Set(on) }
func (o *gpioOutput) Close() error        { return o.line.Close() }

// emulatedOutput prints the LED state instead of driving hardware.
type emulatedOutput struct{}

func (emulatedOutput) Write(on bool) error {
	state := "off"
	if on {
		state = "on"
	}
	fmt.Printf("[emulation] LED %s\n", state)
	return nil
}
func (emulatedOutput) Close() error { return nil }

// -- Inputs ------------------------------------------------------------------

type Button interface {
	ReadPressed() bool
	Close() error
}

type gpioButton struct{ line *common.GpioLine }

func (b *gpioButton) ReadPressed() bool {
	pressed, err := b.line.Get()
	return err == nil && pressed
}
func (b *gpioButton) Close() error { return b.line.Close() }

// noButton never reports a press, so the app is the only thing that
// switches the LED.
type noButton struct{}

func (noButton) ReadPressed() bool { return false }
func (noButton) Close() error      { return nil }

// -- LED state ---------------------------------------------------------------

// LedController owns the LED state — the single source of truth this node
// publishes. Every switch marks the state as pending publication, even one
// that does not change it: a client that guessed wrong about the current
// state would otherwise never be corrected.
type LedController struct {
	mu      sync.Mutex
	output  Output
	on      bool
	pending bool
}

func NewLedController(output Output) *LedController {
	led := &LedController{output: output, pending: true}
	output.Write(false)
	return led
}

func (l *LedController) Set(on bool) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.on = on
	l.output.Write(on)
	l.pending = true
}

func (l *LedController) Toggle() {
	l.mu.Lock()
	on := !l.on
	l.mu.Unlock()
	l.Set(on)
}

func (l *LedController) On() bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	return l.on
}

// TakePending returns true once after each switch, clearing the flag.
func (l *LedController) TakePending() bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	pending := l.pending
	l.pending = false
	return pending
}

// -- Commands ----------------------------------------------------------------

// handleJSONCommand executes one JSON command pushed by the app.
func handleJSONCommand(led *LedController, message []byte) {
	var command map[string]any
	if err := json.Unmarshal(message, &command); err != nil {
		fmt.Println("Ignoring malformed command")
		return
	}
	if command["toggle"] == true {
		fmt.Println("Command: toggle")
		led.Toggle()
		return
	}
	on, ok := command["led"].(bool)
	if !ok {
		fmt.Println("Ignoring command without a boolean 'led'")
		return
	}
	state := "off"
	if on {
		state = "on"
	}
	fmt.Printf("Command: led %s\n", state)
	led.Set(on)
}

// -- State loop --------------------------------------------------------------

// stateLoop polls the button (debounced) and publishes every state change.
// Both sources of change funnel through here — a command handler only
// mutates the controller, and this loop puts the result on the wire, so
// the app sees an app-initiated switch and a button press the same way.
func stateLoop(led *LedController, button Button, publish func(payload any)) {
	var pressed, candidate *bool
	var candidateSince time.Time

	for {
		now := time.Now()
		isPressed := button.ReadPressed()
		if candidate == nil || isPressed != *candidate {
			v := isPressed
			candidate = &v
			candidateSince = now
		} else if (pressed == nil || isPressed != *pressed) && now.Sub(candidateSince) >= debounce {
			firstReading := pressed == nil
			v := isPressed
			pressed = &v
			// The first stable reading only establishes the idle level;
			// toggle on press, not on release, so one press is one toggle.
			if isPressed && !firstReading {
				fmt.Println("Button pressed: toggling LED")
				led.Toggle()
			}
		}

		if led.TakePending() {
			fmt.Printf("led: %v\n", led.On())
			publish(map[string]any{"led": led.On()})
		}
		time.Sleep(pollInterval)
	}
}

// -- Main --------------------------------------------------------------------

func main() {
	buttonActiveLow := true
	cfg := Config{SensorName: "LED", Interface: "gpio", GpioChip: "/dev/gpiochip0", Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	if cfg.ButtonActiveLow != nil {
		buttonActiveLow = *cfg.ButtonActiveLow
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var output Output
	var button Button = noButton{}
	if cfg.Emulation {
		fmt.Println("Emulation mode: switching a virtual LED without hardware")
		output = emulatedOutput{}
	} else {
		if cfg.Interface != "gpio" {
			fmt.Printf("Error: interface %q is not supported in the Go port (only \"gpio\"; use the Python node for Arduino-based hats)\n", cfg.Interface)
			os.Exit(1)
		}
		if cfg.LedPin == nil {
			fmt.Println("Error: config.json: led_pin is required")
			os.Exit(1)
		}
		fmt.Println("Initializing LED node...")
		line, err := common.OpenOutputLine(cfg.GpioChip, *cfg.LedPin, cfg.LedActiveLow)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		output = &gpioOutput{line: line}
		if cfg.ButtonPin != nil {
			bline, err := common.OpenInputLine(cfg.GpioChip, *cfg.ButtonPin, buttonActiveLow)
			if err != nil {
				fmt.Println("Error:", err)
				os.Exit(1)
			}
			button = &gpioButton{line: bline}
		}
	}

	led := NewLedController(output)

	// Leave the LED dark rather than stuck on after the node exits.
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-signals
		led.Set(false)
		output.Close()
		button.Close()
		fmt.Println("Stopped.")
		os.Exit(0)
	}()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey,
		func(send func(payload any) error) {
			send(map[string]any{"led": led.On()})
		},
		func(message []byte) { handleJSONCommand(led, message) },
	)

	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go stateLoop(led, button, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
