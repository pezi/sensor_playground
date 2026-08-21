// Sensor Playground Sensor Node — PAJ7620 Grove Gesture (Go)
//
// Implements the *push* variant of the Sensor Playground Sensor Interface
// on single-board computers (Raspberry Pi & co.) with a Grove Gesture
// sensor (PAJ7620U2). The node polls the sensor over I2C and pushes one
// JSON message ({"gesture": "forward"}) per detected gesture.
//
// The gesture strings match the app's Gesture enum names. Like the
// grove.py driver the Python node uses, the nine basic gestures are
// reported; the combined gestures (forwardBackward, rightLeft, ...) of
// the ESP32 sketch are not detected.
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
//	./sensor_node_paj7620
package main

import (
	"fmt"
	"math/rand"
	"os"
	"time"

	"sensorplayground/common"
)

const pollInterval = 100 * time.Millisecond

type Config struct {
	APIKey    string `json:"api_key"`
	Hostname  string `json:"hostname"`
	I2CBus    int    `json:"i2c_bus"`
	Transport string `json:"transport"`
	Emulation bool   `json:"emulation"`
}

// gestureReader returns the detected gesture name, or "" if nothing
// happened.
type gestureReader interface {
	ReadGesture() (string, error)
}

// emulatedPAJ7620 generates plausible PAJ7620 gestures without hardware:
// one random gesture from the gesture map every three to five seconds;
// polls in between return no gesture (like the Python node).
type emulatedPAJ7620 struct {
	nextAt time.Time
}

func newEmulatedPAJ7620() *emulatedPAJ7620 {
	return &emulatedPAJ7620{nextAt: time.Now().Add(randomGestureGap())}
}

func randomGestureGap() time.Duration {
	return time.Duration((3.0 + 2.0*rand.Float64()) * float64(time.Second))
}

func (e *emulatedPAJ7620) ReadGesture() (string, error) {
	if time.Now().Before(e.nextAt) {
		return "", nil
	}
	e.nextAt = time.Now().Add(randomGestureGap())
	return gestureNames[1+rand.Intn(len(gestureNames))], nil
}

// gestureLoop polls the sensor and pushes each detected gesture.
func gestureLoop(sensor gestureReader, publish func(payload any)) {
	for {
		name, err := sensor.ReadGesture()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			os.Exit(1)
		}
		if name != "" {
			fmt.Printf("Gesture: %s\n", name)
			publish(map[string]any{"gesture": name})
		}
		time.Sleep(pollInterval)
	}
}

func main() {
	cfg := Config{I2CBus: 1, Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var sensor gestureReader
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating PAJ7620 gestures without hardware")
		sensor = newEmulatedPAJ7620()
	} else {
		fmt.Printf("Initializing PAJ7620 gesture sensor on /dev/i2c-%d...\n", cfg.I2CBus)
		real, err := NewPAJ7620(cfg.I2CBus)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		sensor = real
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	server := common.NewWsServer(cfg.APIKey, nil, nil)

	go func() {
		err := common.RunDiscoveryListener("PAJ7620", cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go gestureLoop(sensor, server.Broadcast)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
