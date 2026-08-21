// Sensor Playground Sensor Node — Grove Rotary Angle Sensor (Go, push)
//
// Reads a Grove Rotary Angle Sensor — a 10 kOhm potentiometer with 300° of
// mechanical travel — through the Seeed Grove Base Hat's 12-bit ADC and
// reports the knob position.
// https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/
//
// Unlike the light sensor (also analog, but polled over HTTPS every few
// seconds) this is a *push* node: the app draws a needle that tracks the
// knob, so a reading that is seconds old is useless. The node samples the
// ADC continuously and sends a message whenever the knob has moved further
// than the deadband, plus the current position once per client connect:
//
//	{"adc": 2048, "adcMax": 4095, "angle": 150.1, "angleMax": 300.0}
//
// adcMax travels with every message because the converter's width belongs
// to the board doing the reading, not to the knob: the Grove Base Hat is
// 12-bit (0-4095), the Arduino-based hats the Python node also supports
// ("nano", "grovePlus", both 10-bit) are not implemented in this port.
// `angle`/`angleMax` carry the same position expressed in degrees, so the
// app never needs to know the knob's mechanical travel either.
//
// The Raspberry Pi has no analog input, so an analog sensor needs an
// extension hat with an ADC — there is no direct-GPIO option (unlike the
// digital contact node).
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
//	./sensor_node_rotary_angle
package main

import (
	"fmt"
	"math"
	"os"
	"sync"
	"time"

	"sensorplayground/common"
)

const (
	pollInterval = 40 * time.Millisecond // 25 Hz — matches the ESP32 sketch's publish ceiling

	hatI2CAddress = 0x04 // Grove Base Hat (STM32F030 ADC)
	hatADCBase    = 0x10 // raw 12-bit value registers, one per channel

	// Mechanical travel of the knob, end to end. 300° for the Grove sensor.
	defaultAngleMax = 300.0

	// Default deadband as a fraction of full scale: how far the count must
	// move before a new message goes out. ADC noise jitters the reading by
	// a few counts with the knob at rest, which would otherwise flood the
	// link.
	defaultDeadbandRatio = 0.006 // ~24 counts of 4095
)

// Full-scale ADC count per hat, keyed by hat_type. Only "grove" can be
// read by this port, but the emulation reports the range the configured
// hat really would, like the Python node.
var adcMaxByHat = map[string]int{
	"grove":     4095, // Grove Base Hat, 12-bit STM32F030 ADC
	"nano":      1023, // NanoHat Hub, 10-bit AVR analogRead (BakeBit)
	"grovePlus": 1023, // GrovePi+, 10-bit ATMEGA328P analogRead
}

type Config struct {
	APIKey    string  `json:"api_key"`
	Hostname  string  `json:"hostname"`
	HatType   string  `json:"hat_type"`
	Pin       int     `json:"pin"`
	I2CBus    int     `json:"i2c_bus"`
	AngleMax  float64 `json:"angle_max"`
	Deadband  *int    `json:"deadband"`
	Transport string  `json:"transport"`
	Emulation bool    `json:"emulation"`
}

// -- Sensor ------------------------------------------------------------------

// Sensor reads the knob position as a raw ADC count in [0, adcMax].
type Sensor struct {
	read     func() (int, error)
	adcMax   int
	angleMax float64
}

// newRealSensor reads the raw 12-bit count of the channel through the
// Grove Base Hat, clamped to 0..adcMax.
func newRealSensor(dev *common.I2CDevice, pin int, angleMax float64) *Sensor {
	adcMax := adcMaxByHat["grove"]
	return &Sensor{
		adcMax:   adcMax,
		angleMax: angleMax,
		read: func() (int, error) {
			data, err := dev.ReadRegs(uint8(hatADCBase+pin), 2)
			if err != nil {
				return 0, err
			}
			value := int(data[0]) | int(data[1])<<8 // SMBus words are little-endian
			return max(0, min(adcMax, value)), nil
		},
	}
}

// newEmulatedSensor generates plausible knob movements without hardware:
// it sweeps slowly from end to end and back, as if someone were turning
// the knob by hand, so the app's needle has something to follow.
func newEmulatedSensor(hatType string, angleMax float64) *Sensor {
	// Emulate the range the configured hat would really report, so the
	// app is exercised against a 10-bit node as easily as a 12-bit one.
	adcMax, ok := adcMaxByHat[hatType]
	if !ok {
		adcMax = 4095
	}
	return &Sensor{
		adcMax:   adcMax,
		angleMax: angleMax,
		read: func() (int, error) {
			// A 20-second triangle wave over the full span.
			t := float64(time.Now().UnixNano()) / 1e9
			phase := math.Mod(t, 20.0) / 20.0
			fraction := 1 - math.Abs(2*phase-1)
			return int(math.Round(fraction * float64(adcMax))), nil
		},
	}
}

// payloadFor builds the position message. The scale travels with every
// reading.
func payloadFor(sensor *Sensor, adc int) map[string]any {
	angle := float64(adc) / float64(sensor.adcMax) * sensor.angleMax
	return map[string]any{
		"adc":      adc,
		"adcMax":   sensor.adcMax,
		"angle":    math.Round(angle*10) / 10,
		"angleMax": sensor.angleMax,
	}
}

// defaultDeadband scales the deadband to this hat's range, so a 10-bit
// node is not held to a 12-bit node's precision.
func defaultDeadband(adcMax int) int {
	return max(1, int(math.Ceil(float64(adcMax)*defaultDeadbandRatio)))
}

// shouldPublish decides whether a fresh count goes on the wire: the first
// reading and any move past the deadband — and the ends of travel are
// pinned, so a knob turned fully reports exactly 0 or full scale instead
// of stopping a deadband short of it.
func shouldPublish(published *int, adc, adcMax, deadband int) bool {
	if published == nil {
		return true
	}
	if abs(adc-*published) >= deadband {
		return true
	}
	return (adc == 0 || adc == adcMax) && adc != *published
}

func abs(x int) int {
	if x < 0 {
		return -x
	}
	return x
}

// -- State -------------------------------------------------------------------

// Latest published count, shared with newly connected clients (nil until
// the first sample).
var (
	stateMu sync.Mutex
	lastADC *int
)

func setLastADC(adc int) {
	stateMu.Lock()
	defer stateMu.Unlock()
	lastADC = &adc
}

func currentLastADC() (adc int, known bool) {
	stateMu.Lock()
	defer stateMu.Unlock()
	if lastADC == nil {
		return 0, false
	}
	return *lastADC, true
}

// -- Sample loop -------------------------------------------------------------

// rotaryLoop samples the knob and publishes whenever it moves past
// [deadband].
func rotaryLoop(sensor *Sensor, publish func(payload any), deadband int) {
	var published *int

	for {
		adc, err := sensor.read()
		if err != nil {
			fmt.Println("Sensor read failed:", err)
			time.Sleep(pollInterval)
			continue
		}
		if shouldPublish(published, adc, sensor.adcMax, deadband) {
			v := adc
			published = &v
			setLastADC(adc)
			payload := payloadFor(sensor, adc)
			fmt.Printf("adc: %d/%d  angle: %v\n", adc, sensor.adcMax, payload["angle"])
			publish(payload)
		}
		time.Sleep(pollInterval)
	}
}

// -- Main --------------------------------------------------------------------

func main() {
	cfg := Config{HatType: "grove", I2CBus: 1, AngleMax: defaultAngleMax, Transport: "wifi"}
	if err := common.LoadJSONConfig(&cfg); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
	if cfg.APIKey == "" {
		fmt.Println("Error: config.json: api_key is required")
		os.Exit(1)
	}
	cfg.Hostname = common.HostnameOr(cfg.Hostname)

	var sensor *Sensor
	if cfg.Emulation {
		fmt.Println("Emulation mode: generating ROTARY readings without hardware")
		sensor = newEmulatedSensor(cfg.HatType, cfg.AngleMax)
	} else {
		if cfg.HatType != "grove" {
			fmt.Printf("Error: hat_type %q is not supported in the Go port (only \"grove\"; use the Python node for Arduino-based hats)\n", cfg.HatType)
			os.Exit(1)
		}
		if cfg.Pin < 0 || cfg.Pin > 7 {
			fmt.Printf("Error: invalid channel %d - valid range [0,7]\n", cfg.Pin)
			os.Exit(1)
		}
		fmt.Printf("Initializing Grove Rotary Angle Sensor on grove hat, channel %d...\n", cfg.Pin)
		dev, err := common.OpenI2C(cfg.I2CBus, hatI2CAddress)
		if err != nil {
			fmt.Println("Error:", err)
			os.Exit(1)
		}
		sensor = newRealSensor(dev, cfg.Pin, cfg.AngleMax)
	}

	fmt.Printf("ADC range: 0-%d, travel %.0f deg\n", sensor.adcMax, sensor.angleMax)

	// A deadband given in counts wins; otherwise scale it to this hat's
	// range.
	deadband := defaultDeadband(sensor.adcMax)
	if cfg.Deadband != nil {
		deadband = *cfg.Deadband
	}

	if cfg.Transport == "ble" {
		fmt.Println("Warning: BLE transport is not supported in the Go port, using wifi.")
	}

	// Send the current position to a client right after it connects.
	server := common.NewWsServer(cfg.APIKey,
		func(send func(payload any) error) {
			adc, known := currentLastADC()
			if !known {
				fresh, err := sensor.read()
				if err != nil {
					return
				}
				adc = fresh
			}
			send(payloadFor(sensor, adc))
		},
		nil,
	)

	go func() {
		err := common.RunDiscoveryListener("ROTARY", cfg.Hostname, common.WSPort, nil)
		if err != nil {
			fmt.Println("UDP discovery failed:", err)
		}
	}()
	go rotaryLoop(sensor, server.Broadcast, deadband)
	if err := server.ListenAndServe(); err != nil {
		fmt.Println("Error:", err)
		os.Exit(1)
	}
}
