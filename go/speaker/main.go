// Sensor Playground Speaker Node — Grove Speaker (Go)
//
// Implements the *actuator* variant of the Sensor Playground Sensor
// Interface on single-board computers (Raspberry Pi & co.) with a Grove
// Speaker — a small amplified loudspeaker on a digital pin, driven with a
// square wave of the desired pitch. Like the LED the node talks in both
// directions: the app asks for a tone, the built-in melody, or silence,
// and the node reports what is *actually* sounding — a tone ends on its
// own when its duration runs out, so the app follows the node's reports
// rather than its own taps.
//
//		app -> node   {"tone": {"freq": 440, "ms": 400}}   play one tone
//		              {"melody": true}                     play the built-in melody
//		              {"stop": true}                       silence
//		node -> app   {"freq": 440} / {"freq": 0}          what is sounding (on
//		                                                   connect and on every
//		                                                   change)
//
//	  - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//	  - BLE is not supported in this port; "transport": "ble" falls back to
//	    the WebSocket with a warning (use the Python or Rust node for BLE).
//
// Set "emulation": true to run without any hardware at all.
//
// Usage:
//
//	cp config.example.json config.json   # edit with your settings
//	./sensor_node_speaker
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"os/signal"
	"sync"
	"syscall"
	"time"

	"sensorplayground/common"
)

// How often the playback loop advances tones and publishes state changes.
const pollInterval = 20 * time.Millisecond

// Accepted tone range; anything else is ignored as noise.
const (
	minFreqHz = 20
	maxFreqHz = 20000
)

// note is one step of a melody.
type note struct {
	frequency int
	duration  time.Duration
}

// melody is the built-in tune: a little C-major fanfare.
var melody = []note{
	{262, 250 * time.Millisecond}, {330, 250 * time.Millisecond},
	{392, 250 * time.Millisecond}, {523, 350 * time.Millisecond},
	{392, 250 * time.Millisecond}, {330, 250 * time.Millisecond},
	{262, 500 * time.Millisecond},
}

type Config struct {
	APIKey     string `json:"api_key"`
	Hostname   string `json:"hostname"`
	SensorName string `json:"sensor_name"`
	SpeakerPin int    `json:"speaker_pin"`
	GpioChip   int    `json:"gpio_chip"`
	Transport  string `json:"transport"`
	Emulation  bool   `json:"emulation"`
}

// -- Playback state ------------------------------------------------------

// SpeakerController owns the sounding state — the single source of truth
// this node publishes.
//
// Command handlers only stage playback; Tick() — driven by the serving
// loop — is what advances melodies and ends tones on time, and every
// change marks the state as pending publication.
type SpeakerController struct {
	mu          sync.Mutex
	output      Output
	frequency   int
	pending     bool
	deadline    time.Time
	hasDeadline bool
	remaining   []note // remaining melody steps
}

func NewSpeakerController(output Output) *SpeakerController {
	c := &SpeakerController{
		output:  output,
		pending: true, // publish the initial state as soon as we serve
	}
	c.output.Play(0)
	return c
}

// apply starts one note. The caller holds the lock.
func (c *SpeakerController) apply(frequency int, duration time.Duration) {
	c.frequency = frequency
	c.hasDeadline = frequency > 0
	if c.hasDeadline {
		c.deadline = time.Now().Add(duration)
	}
	c.output.Play(frequency)
	// Publish unconditionally: a redundant command from a client that
	// guessed wrong would otherwise never be corrected.
	c.pending = true
}

// PlayTone plays one tone, cancelling any melody.
func (c *SpeakerController) PlayTone(frequency, milliseconds int) {
	c.mu.Lock()
	defer c.mu.Unlock()
	if frequency < minFreqHz || frequency > maxFreqHz || milliseconds <= 0 {
		fmt.Printf("Ignoring tone %d Hz / %d ms\n", frequency, milliseconds)
		return
	}
	fmt.Printf("Tone: %d Hz for %d ms\n", frequency, milliseconds)
	c.remaining = nil
	c.apply(frequency, time.Duration(milliseconds)*time.Millisecond)
}

// PlayMelody starts the built-in melody from its first note.
func (c *SpeakerController) PlayMelody() {
	c.mu.Lock()
	defer c.mu.Unlock()
	fmt.Println("Melody")
	c.remaining = append([]note(nil), melody[1:]...)
	c.apply(melody[0].frequency, melody[0].duration)
}

// Stop silences the speaker, whatever it is playing.
func (c *SpeakerController) Stop() {
	c.mu.Lock()
	defer c.mu.Unlock()
	fmt.Println("Stop")
	c.remaining = nil
	c.apply(0, 0)
}

// Tick ends a finished tone, or steps through the melody.
func (c *SpeakerController) Tick() {
	c.mu.Lock()
	defer c.mu.Unlock()
	if !c.hasDeadline || time.Now().Before(c.deadline) {
		return
	}
	if len(c.remaining) > 0 {
		next := c.remaining[0]
		c.remaining = c.remaining[1:]
		c.apply(next.frequency, next.duration)
		return
	}
	c.apply(0, 0)
}

// TakePending returns true once after each change, clearing the flag.
func (c *SpeakerController) TakePending() bool {
	c.mu.Lock()
	defer c.mu.Unlock()
	pending := c.pending
	c.pending = false
	return pending
}

// State returns the state payload the transports publish.
func (c *SpeakerController) State() map[string]any {
	c.mu.Lock()
	defer c.mu.Unlock()
	return map[string]any{"freq": c.frequency}
}

func (c *SpeakerController) Close() {
	c.mu.Lock()
	defer c.mu.Unlock()
	c.output.Close()
}

// -- Commands ------------------------------------------------------------

// handleJSONCommand executes one JSON command pushed by the app over the
// WebSocket.
func handleJSONCommand(speaker *SpeakerController, message []byte) {
	var command map[string]any
	if err := json.Unmarshal(message, &command); err != nil {
		fmt.Println("Ignoring malformed command")
		return
	}

	if stop, _ := command["stop"].(bool); stop {
		speaker.Stop()
		return
	}
	if play, _ := command["melody"].(bool); play {
		speaker.PlayMelody()
		return
	}
	// JSON numbers decode as float64; accept integers only, like the
	// Python node's isinstance(int) check.
	if tone, isObject := command["tone"].(map[string]any); isObject {
		frequency, freqIsNumber := tone["freq"].(float64)
		milliseconds, msIsNumber := tone["ms"].(float64)
		if freqIsNumber && msIsNumber &&
			frequency == math.Trunc(frequency) && milliseconds == math.Trunc(milliseconds) {
			speaker.PlayTone(int(frequency), int(milliseconds))
			return
		}
	}
	fmt.Println("Ignoring unknown command")
}

// -- Playback loop -------------------------------------------------------

// playbackLoop advances playback and publishes every state change.
//
// Command handlers only mutate the controller; this loop ends tones on
// time and puts the resulting states on the wire, so the app sees a
// command echo and a tone that ran out the same way.
func playbackLoop(speaker *SpeakerController, publish func(payload any)) {
	for {
		speaker.Tick()
		if speaker.TakePending() {
			publish(speaker.State())
		}
		time.Sleep(pollInterval)
	}
}

// -- Main ----------------------------------------------------------------

func main() {
	cfg := Config{SensorName: "SPEAKER", SpeakerPin: 5, Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var output Output
	if cfg.Emulation {
		fmt.Println("Emulation mode: printing tones without hardware")
		output = &EmulatedOutput{}
	} else {
		fmt.Printf("Initializing speaker on GPIO %d...\n", cfg.SpeakerPin)
		pwm, err := NewPwmOutput(cfg.GpioChip, cfg.SpeakerPin)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		output = pwm
	}
	speaker := NewSpeakerController(output)

	// Silence the speaker rather than leaving a tone sounding forever.
	signals := make(chan os.Signal, 1)
	signal.Notify(signals, os.Interrupt, syscall.SIGTERM)
	go func() {
		<-signals
		speaker.Close()
		fmt.Println("Stopped.")
		os.Exit(0)
	}()

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey,
		func(send func(payload any) error) { send(speaker.State()) },
		func(message []byte) { handleJSONCommand(speaker, message) },
	)

	go func() {
		err := common.RunDiscoveryListener(cfg.SensorName, cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go playbackLoop(speaker, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
