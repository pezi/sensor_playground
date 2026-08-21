// Compact TSL2591 driver — a port of the `adafruit_tsl2591` library the
// Python node uses: the same device-ID check, the same gain and
// integration-time encoding, and the same two-equation lux formula from
// the Adafruit Arduino library, so the values match the Python node.
//
// Every register access ORs the address with the command bit 0xA0 (command
// + normal operation), exactly as the Python library does.
//
// The lux maths is platform-neutral (and unit tested); only the I2C access
// needs hardware.
package main

import (
	"fmt"

	"sensorplayground/common"
)

// -- TSL2591 register map -------------------------------------------------

const (
	tslCommandBit = 0xA0 // every register access is OR'd with this

	tslRegEnable   = 0x00
	tslRegControl  = 0x01
	tslRegDeviceID = 0x12
	tslRegChan0Low = 0x14 // broadband (visible + IR), then channel 1 at 0x16

	tslEnablePowerOn = 0x01
	tslEnableAEN     = 0x02 // ALS enable

	tslDeviceID = 0x50

	// The ADC is 16-bit, but at the shortest integration time it only
	// counts to 0x8FFF.
	tslMaxCount100ms = 0x8FFF
	tslMaxCount      = 0xFFFF

	// Lux equation coefficients (Adafruit Arduino library).
	tslLuxDF    = 408.0
	tslLuxCoefB = 1.64
	tslLuxCoefC = 0.59
	tslLuxCoefD = 0.86
)

// Counts at which the current gain is judged too high or too low. A
// saturated channel reads 0xFFFF and the lux value is invalid.
const (
	saturationCounts = 0xFFFF
	tooDarkCounts    = 100
)

// gainNames are the values accepted in config.json, in ascending order;
// the index is the position in gainRegisters/gainFactors.
var (
	gainNames     = [4]string{"low", "med", "high", "max"}
	gainRegisters = [4]uint8{0x00, 0x10, 0x20, 0x30}
	gainFactors   = [4]float64{1.0, 25.0, 428.0, 9876.0}
)

// integrationRegister maps an integration time in milliseconds to the
// register value (0..5); the chip only supports these six.
var integrationRegister = map[int]uint8{100: 0, 200: 1, 300: 2, 400: 3, 500: 4, 600: 5}

// gainIndexFor resolves a config gain name to its index.
func gainIndexFor(name string) (int, error) {
	for index, candidate := range gainNames {
		if candidate == name {
			return index, nil
		}
	}
	return 0, fmt.Errorf("gain must be one of low, med, high, max, got %q", name)
}

// calculateLux converts the two raw channel counts to lux, given the
// integration time register value and the gain factor. ok is false when a
// channel saturated: the counts still show the app that it is very bright,
// but the lux value would be wrong.
func calculateLux(channel0, channel1 uint16, integrationRegisterValue uint8, again float64) (lux float64, ok bool) {
	atime := 100.0*float64(integrationRegisterValue) + 100.0
	maxCounts := uint16(tslMaxCount)
	if integrationRegisterValue == 0 {
		maxCounts = tslMaxCount100ms
	}
	if channel0 >= maxCounts || channel1 >= maxCounts {
		return 0, false
	}
	c0, c1 := float64(channel0), float64(channel1)
	cpl := (atime * again) / tslLuxDF
	// Two approximations of the visible response; the library takes
	// whichever is larger.
	lux1 := (c0 - tslLuxCoefB*c1) / cpl
	lux2 := (tslLuxCoefC*c0 - tslLuxCoefD*c1) / cpl
	return max(lux1, lux2), true
}

// -- Hardware -------------------------------------------------------------

type TSL2591 struct {
	dev              *common.I2CDevice
	gainIndex        int
	integrationValue uint8
	autoGain         bool
}

// NewTSL2591 opens the sensor on /dev/i2c-<bus> at 0x29, verifies the
// device ID and programs the configured gain and integration time.
func NewTSL2591(bus int, gain string, integrationMs int, autoGain bool) (*TSL2591, error) {
	gainIndex, err := gainIndexFor(gain)
	if err != nil {
		return nil, err
	}
	integrationValue, valid := integrationRegister[integrationMs]
	if !valid {
		return nil, fmt.Errorf("integration_ms must be one of 100, 200, 300, 400, 500, 600, got %d",
			integrationMs)
	}
	dev, err := common.OpenI2C(bus, 0x29)
	if err != nil {
		return nil, err
	}
	s := &TSL2591{dev: dev, gainIndex: gainIndex, integrationValue: integrationValue, autoGain: autoGain}
	deviceID, err := s.readU8(tslRegDeviceID)
	if err != nil {
		return nil, err
	}
	if deviceID != tslDeviceID {
		return nil, fmt.Errorf("no TSL2591 at 0x29 (device ID 0x%02x, expected 0x%02x)",
			deviceID, tslDeviceID)
	}
	if err := s.applyGain(); err != nil {
		return nil, err
	}
	if err := s.applyIntegration(); err != nil {
		return nil, err
	}
	// Power on and enable the ALS; the chip then integrates continuously.
	return s, s.writeU8(tslRegEnable, tslEnablePowerOn|tslEnableAEN)
}

func (s *TSL2591) Close() error { return s.dev.Close() }

func (s *TSL2591) readU8(reg uint8) (uint8, error) {
	return s.dev.ReadReg(tslCommandBit | reg)
}

func (s *TSL2591) writeU8(reg, value uint8) error {
	return s.dev.WriteReg(tslCommandBit|reg, value)
}

// applyGain writes the gain bits, leaving the integration bits alone.
func (s *TSL2591) applyGain() error {
	control, err := s.readU8(tslRegControl)
	if err != nil {
		return err
	}
	return s.writeU8(tslRegControl, control&0b11001111|gainRegisters[s.gainIndex])
}

// applyIntegration writes the integration bits, leaving the gain alone.
func (s *TSL2591) applyIntegration() error {
	control, err := s.readU8(tslRegControl)
	if err != nil {
		return err
	}
	return s.writeU8(tslRegControl, control&0b11111000|s.integrationValue)
}

// RawLuminosity returns the broadband (visible + IR) and infrared counts.
func (s *TSL2591) RawLuminosity() (broadband, infrared uint16, err error) {
	data, err := s.dev.ReadRegs(tslCommandBit|tslRegChan0Low, 4)
	if err != nil {
		return 0, 0, err
	}
	return uint16(data[0]) | uint16(data[1])<<8, uint16(data[2]) | uint16(data[3])<<8, nil
}

// Lux converts the two channel counts with the currently programmed gain
// and integration time.
func (s *TSL2591) Lux(broadband, infrared uint16) (float64, bool) {
	return calculateLux(broadband, infrared, s.integrationValue, gainFactors[s.gainIndex])
}

// AutoGainStep moves one gain step when the broadband channel pins at
// either end.
//
// Only one step per reading: changing the gain invalidates the integration
// already in flight, so the *next* reading is the one that benefits.
// Jumping straight to the extreme instead would make the value oscillate
// whenever the light sits near a threshold.
func (s *TSL2591) AutoGainStep(broadband uint16) {
	if !s.autoGain {
		return
	}
	index := s.gainIndex
	if broadband >= saturationCounts {
		index = max(0, index-1)
	} else if broadband <= tooDarkCounts {
		index = min(len(gainFactors)-1, index+1)
	}
	if index != s.gainIndex {
		s.gainIndex = index
		if err := s.applyGain(); err != nil {
			fmt.Println("Adjusting gain failed:", err)
		}
	}
}
