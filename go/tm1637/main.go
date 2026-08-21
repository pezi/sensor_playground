// Sensor Playground Clock Node — Grove 4-Digit Display / TM1637 (Go)
//
// Implements the *actuator* variant of the Sensor Playground Sensor
// Interface: the app pushes the time to show (and a brightness), and the
// node reports the state it is actually displaying — which keeps changing
// on its own, because once a time is set the node advances the minute and
// blinks the colon autonomously.
//
//	app -> node   {"time": "HH:MM"}     set the displayed time (24-hour)
//	node -> app   {"time": "12:34", "brightness": 3}   current state
//	              {"time": null, "brightness": 3}      no time set yet
//
// The node pushes its state on connect, after every accepted command, and
// on each minute rollover — never on the colon blink, so state traffic
// stays at one message a minute. Before the first time set the display
// shows "--:--".
//
// The node owns the state; the app renders what the node last reported
// rather than what it asked for, so a command that never arrived cannot
// leave the app showing a time the display does not.
//
// The TM1637 bus is bit-banged on two GPIO character-device lines
// ("interface": "gpio", /dev/gpiochipN — /sys/class/gpio is gone in
// Debian 13). The Arduino-based extension hats ("interface": "hat") are
// not supported for this node: one frame needs ~50 line transitions and
// each hat digital_write is a full I2C transaction, far too slow for a
// display bus.
//
//   - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//   - BLE is not supported in this port; "transport": "ble" falls back to
//     the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true to run without any hardware at all.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_tm1637
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"os/signal"
	"regexp"
	"sync"
	"syscall"
	"time"

	"sensorplayground/common"
)

const (
	pollInterval      = 50 * time.Millisecond // 20 Hz clock/blink tick
	colonBlink        = 500 * time.Millisecond
	defaultBrightness = 3
)

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	SensorName string `json:"sensor_name"`
	Interface  string `json:"interface"`
	GpioChip   string `json:"gpio_chip"`
	ClkPin     int    `json:"clk_pin"`
	DioPin     int    `json:"dio_pin"`
	Brightness int    `json:"brightness"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

// -- Displays ----------------------------------------------------------------

type Display interface {
	Render(set bool, hour, minute int, colonOn bool, brightness int)
	Close()
}

// EmulatedDisplay prints the displayed state instead of driving hardware.
// Prints on time/brightness changes and minute rollovers only — the colon
// blink would flood the console at 1 Hz.
type EmulatedDisplay struct {
	lastPrinted string
}

func (d *EmulatedDisplay) Render(set bool, hour, minute int, colonOn bool, brightness int) {
	text := "--:--"
	if set {
		text = fmt.Sprintf("%02d:%02d", hour, minute)
	}
	shown := fmt.Sprintf("%s/%d", text, brightness)
	if shown == d.lastPrinted {
		return
	}
	d.lastPrinted = shown
	fmt.Printf("[emulation] display [%s] brightness=%d\n", text, brightness)
}

func (d *EmulatedDisplay) Close() {}

func makeDisplay(cfg *Config) Display {
	if cfg.Emulation {
		return &EmulatedDisplay{}
	}
	switch cfg.Interface {
	case "gpio":
		display, err := NewGpioDisplay(cfg.GpioChip, cfg.ClkPin, cfg.DioPin)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		return display
	case "hat":
		fmt.Println("Error: the Arduino-based extension hats cannot drive a TM1637: one " +
			"frame needs ~50 line transitions and each hat digital_write is " +
			"a full I2C transaction. Wire the display to Pi GPIOs and use " +
			"\"gpio\" (a Grove Base Hat's digital ports work — they are wired " +
			"straight to the Pi).")
	default:
		fmt.Printf("Error: unknown interface %q (use \"gpio\")\n", cfg.Interface)
	}
	os.Exit(1)
	return nil
}

// -- Clock state --------------------------------------------------------------

// ClockController owns the displayed clock state — the single source of
// truth this node publishes — and keeps it ticking.
//
// Every command marks the state as pending publication, even one that
// does not change it: a client that guessed wrong about the current state
// would otherwise never be corrected. The colon blink renders but never
// marks pending; the minute rollover does both.
type ClockController struct {
	mu          sync.Mutex
	display     Display
	set         bool
	hour        int
	minute      int
	brightness  int
	pending     bool
	colonOn     bool
	lastBlink   time.Time
	minuteAccum float64
	lastTick    time.Time
}

func NewClockController(display Display, brightness int) *ClockController {
	c := &ClockController{
		display:    display,
		brightness: clampBrightness(brightness),
		pending:    true, // publish the initial state as soon as we serve
	}
	c.render()
	return c
}

func clampBrightness(brightness int) int {
	return max(0, min(7, brightness))
}

func (c *ClockController) render() {
	c.display.Render(c.set, c.hour, c.minute, c.colonOn, c.brightness)
}

// SetTime sets the displayed time (24-hour) and restarts the minute phase.
func (c *ClockController) SetTime(hour, minute int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.set = true
	c.hour = hour
	c.minute = minute
	// ":00 seconds" is now, and a lit colon gives immediate feedback.
	c.minuteAccum = 0
	c.colonOn = true
	c.lastBlink = time.Now()
	c.render()
	c.pending = true
}

// SetBrightness sets the display brightness, clamped to 0..7.
func (c *ClockController) SetBrightness(brightness int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.brightness = clampBrightness(brightness)
	c.render()
	c.pending = true
}

// RequestState marks the state for re-publication without changing it.
func (c *ClockController) RequestState() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.pending = true
}

// Tick advances the local clock and blinks the colon. Only the minute
// rollover marks the state pending; the blink is render-only.
func (c *ClockController) Tick() {
	c.mu.Lock()
	defer c.mu.Unlock()

	now := time.Now()
	if c.lastTick.IsZero() {
		c.lastTick = now
	}
	elapsed := now.Sub(c.lastTick).Seconds()
	c.lastTick = now

	if !c.set {
		return
	}

	if now.Sub(c.lastBlink) >= colonBlink {
		c.lastBlink = now
		c.colonOn = !c.colonOn
		c.render()
	}

	c.minuteAccum += elapsed
	if c.minuteAccum < 60.0 {
		return
	}
	c.minuteAccum -= 60.0
	c.minute++
	if c.minute >= 60 {
		c.minute = 0
		c.hour++
		if c.hour >= 24 { // midnight rollover
			c.hour = 0
		}
	}
	c.render()
	c.pending = true
}

// TakePending returns true once after each change, clearing the flag.
func (c *ClockController) TakePending() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	pending := c.pending
	c.pending = false
	return pending
}

// State returns the state payload the transports publish.
func (c *ClockController) State() map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	var timeText any
	if c.set {
		timeText = fmt.Sprintf("%02d:%02d", c.hour, c.minute)
	}
	return map[string]any{"time": timeText, "brightness": c.brightness}
}

func (c *ClockController) Close() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.display.Close()
}

// -- Commands ----------------------------------------------------------------

var timePattern = regexp.MustCompile(`^(\d{2}):(\d{2})$`)

// parseTimeText parses an "HH:MM" string, returning (hour, minute, ok).
func parseTimeText(text string) (int, int, bool) {
	match := timePattern.FindStringSubmatch(text)
	if match == nil {
		return 0, 0, false
	}
	hour := int(match[1][0]-'0')*10 + int(match[1][1]-'0')
	minute := int(match[2][0]-'0')*10 + int(match[2][1]-'0')
	if hour > 23 || minute > 59 {
		return 0, 0, false
	}
	return hour, minute, true
}

// handleJSONCommand executes one JSON command pushed by the app over the
// WebSocket.
func handleJSONCommand(clock *ClockController, message []byte) {
	var command map[string]any
	if err := json.Unmarshal(message, &command); err != nil {
		fmt.Println("Ignoring malformed command")
		return
	}

	if raw, present := command["time"]; present {
		text, _ := raw.(string)
		hour, minute, ok := parseTimeText(text)
		if !ok {
			fmt.Println("Ignoring invalid time")
			return
		}
		fmt.Printf("Command: time %02d:%02d\n", hour, minute)
		clock.SetTime(hour, minute)
		return
	}

	// JSON numbers decode as float64; accept integers only, like the
	// Python node's isinstance(int) check (booleans are a distinct type
	// in Go's decoding already).
	brightness, isNumber := command["brightness"].(float64)
	if !isNumber || brightness != math.Trunc(brightness) {
		fmt.Println("Ignoring unknown command")
		return
	}
	fmt.Printf("Command: brightness %d\n", int(brightness))
	clock.SetBrightness(int(brightness))
}

// -- State loop --------------------------------------------------------------

// stateLoop ticks the clock and publishes every pending state. All
// sources of change funnel through here — a command handler only mutates
// the controller and this loop is what puts the result on the wire, so
// the app sees a command echo and a minute rollover the same way.
func stateLoop(clock *ClockController, publish func(payload any)) {
	for {
		clock.Tick()
		if clock.TakePending() {
			state := clock.State()
			encoded, _ := json.Marshal(state)
			fmt.Printf("state: %s\n", encoded)
			publish(state)
		}
		time.Sleep(pollInterval)
	}
}

// -- Main --------------------------------------------------------------------

func main() {
	cfg := Config{
		SensorName: "TM1637",
		Interface:  "gpio",
		GpioChip:   "/dev/gpiochip0",
		ClkPin:     5,
		DioPin:     6,
		Brightness: defaultBrightness,
		Transport:  "wifi",
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
		fmt.Println("Emulation mode: driving a virtual display without hardware")
	} else {
		fmt.Println("Initializing TM1637 clock node...")
	}
	clock := NewClockController(makeDisplay(&cfg), cfg.Brightness)

	// Blank the panel rather than leaving a stale time burning.
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-signals
		clock.Close()
		fmt.Println("Stopped.")
		os.Exit(0)
	}()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey,
		func(send func(payload any) error) {
			send(clock.State())
		},
		func(message []byte) { handleJSONCommand(clock, message) },
	)

	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go stateLoop(clock, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
