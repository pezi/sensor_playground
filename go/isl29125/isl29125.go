// Compact ISL29125 driver, ported from the register access in the Python
// node (smbus2): the device-ID check and reset, the three configuration
// registers, and a 6-byte block read of the green/red/blue counts starting
// at register 0x09 — readings match the Python node.
//
// The colour derivation below is platform-neutral (and unit tested); only
// the I2C access needs hardware.
package main

import (
	"fmt"
	"math"
	"time"

	"sensorplayground/common"
)

// -- ISL29125 register map (see the SparkFun library / datasheet) ----------

const (
	islRegDeviceID = 0x00 // reads 0x7D; writing 0x46 resets the chip
	islRegConfig1  = 0x01
	islRegConfig2  = 0x02
	islRegConfig3  = 0x03
	islRegGreenLow = 0x09 // G L/H, R L/H, B L/H — six consecutive bytes

	islDeviceID     = 0x7D
	islResetCommand = 0x46

	// CONFIG1: RGB sampling mode (0x05) in the 10,000 lux range (0x08), 16-bit.
	islConfig1RGB10kLux = 0x0D
	// CONFIG2: IR compensation on, maximum adjustment (the SparkFun default).
	islConfig2IRAdjustHigh = 0xBF
	islConfig3NoInterrupts = 0x00
)

// Approximate green-counts-to-lux factor for the 10K range at 16 bits.
const luxPerCount = 10000.0 / 65535.0

// -- Colour derivation ----------------------------------------------------

// deriveReading turns the raw 16-bit channel counts into the REST payload:
// an approximate illuminance from the green channel, whose spectral
// response resembles the human eye, plus the colour normalized against the
// brightest channel so the app can show it directly. In complete darkness
// there is no colour to report, so the red/green/blue keys are absent.
func deriveReading(green, red, blue uint16) map[string]any {
	reading := map[string]any{"lux": int(math.Round(float64(green) * luxPerCount))}
	brightest := green
	if red > brightest {
		brightest = red
	}
	if blue > brightest {
		brightest = blue
	}
	if brightest > 0 {
		normalize := func(channel uint16) int {
			return int(math.Round(255 * float64(channel) / float64(brightest)))
		}
		reading["red"] = normalize(red)
		reading["green"] = normalize(green)
		reading["blue"] = normalize(blue)
	}
	return reading
}

// -- Hardware -------------------------------------------------------------

type ISL29125 struct {
	dev *common.I2CDevice
}

// NewISL29125 opens the sensor on /dev/i2c-<bus>, verifies the device ID
// and configures RGB sampling.
func NewISL29125(bus int, address uint8) (*ISL29125, error) {
	dev, err := common.OpenI2C(bus, address)
	if err != nil {
		return nil, err
	}
	deviceID, err := dev.ReadReg(islRegDeviceID)
	if err != nil {
		return nil, err
	}
	if deviceID != islDeviceID {
		return nil, fmt.Errorf("No ISL29125 at 0x%02x (device ID 0x%02x, expected 0x%02x)",
			address, deviceID, islDeviceID)
	}
	if err := dev.WriteReg(islRegDeviceID, islResetCommand); err != nil {
		return nil, err
	}
	time.Sleep(100 * time.Millisecond)
	for _, w := range []struct{ reg, val uint8 }{
		{islRegConfig1, islConfig1RGB10kLux},
		{islRegConfig2, islConfig2IRAdjustHigh},
		{islRegConfig3, islConfig3NoInterrupts},
	} {
		if err := dev.WriteReg(w.reg, w.val); err != nil {
			return nil, err
		}
	}
	// One conversion takes ~100 ms per channel; let the first RGB sampling
	// cycle complete before serving readings.
	time.Sleep(400 * time.Millisecond)
	return &ISL29125{dev: dev}, nil
}

func (s *ISL29125) Close() error { return s.dev.Close() }

// ReadChannels returns the raw 16-bit green, red and blue counts.
func (s *ISL29125) ReadChannels() (green, red, blue uint16, err error) {
	data, err := s.dev.ReadRegs(islRegGreenLow, 6)
	if err != nil {
		return 0, 0, 0, err
	}
	green = uint16(data[0]) | uint16(data[1])<<8
	red = uint16(data[2]) | uint16(data[3])<<8
	blue = uint16(data[4]) | uint16(data[5])<<8
	return green, red, blue, nil
}
