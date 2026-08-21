// Compact TCS34725 driver — a port of the `adafruit_tcs34725` library the
// Python node uses: the same sensor-ID check, the same 154 ms / 4x profile,
// the same gamma-corrected RGB bytes, and the same DN40 lux/colour
// temperature algorithm, so the values match the Python node.
//
// Every register access ORs the address with the command bit 0x80, exactly
// as the Python library does — that is the byte sequence the reference
// node puts on the wire, block reads included.
//
// The colour maths is platform-neutral (and unit tested); only the I2C
// access needs hardware.
package main

import (
	"fmt"
	"math"
	"time"

	"sensorplayground/common"
)

// -- TCS34725 register map ------------------------------------------------

const (
	tcsCommandBit = 0x80 // every register access is OR'd with this

	tcsRegEnable   = 0x00
	tcsRegATime    = 0x01
	tcsRegControl  = 0x0F
	tcsRegSensorID = 0x12
	tcsRegStatus   = 0x13
	tcsRegCData    = 0x14 // C, R, G, B — eight consecutive bytes, LE words

	tcsEnableAEN = 0x02 // ADC enable
	tcsEnablePON = 0x01 // power on

	tcsStatusAValid = 0x01 // a conversion has completed
)

// The gain register holds the *index* into this table.
var tcsGains = [4]float64{1, 4, 16, 60}

// DN40 device-specific values (DN40 Table 1, Appendix I).
const (
	dn40GlassAttenuation = 1.0
	dn40DeviceFactor     = 310.0
	dn40RCoef            = 0.136
	dn40GCoef            = 1.0
	dn40BCoef            = -0.444
	dn40CTCoef           = 3810.0
	dn40CTOffset         = 1391.0
)

// -- Colour maths ---------------------------------------------------------

// colorRGBBytes normalizes the red, green and blue counts against the clear
// channel and applies the library's 2.5 gamma correction, giving the 0-255
// bytes the app paints. Complete darkness (clear == 0) is black.
func colorRGBBytes(r, g, b, clear uint16) (int, int, int) {
	if clear == 0 {
		return 0, 0, 0
	}
	channel := func(value uint16) int {
		// int() truncation at both steps, like the Python library.
		normalized := float64(int(float64(value)/float64(clear)*256)) / 255.0
		return min(int(math.Pow(normalized, 2.5)*255), 255)
	}
	return channel(r), channel(g), channel(b)
}

// temperatureAndLuxDN40 converts raw R/G/B/C counts to illuminance and
// colour temperature with the algorithm from Taos/AMS design note DN40.
// ok is false when the clear channel saturated: the sample says nothing
// about the colour then, so the node reports no reading at all.
func temperatureAndLuxDN40(r, g, b, c uint16, atimeMs, again float64) (lux, colorTemperature float64, ok bool) {
	// Analog/digital saturation (DN40 3.5). The ATIME register holds
	// 256 - cycles, so cycles is what the count limit scales with.
	cycles := math.Round(atimeMs / 2.4)
	saturation := 65535.0
	if cycles <= 63 {
		saturation = 1024 * cycles
	}
	// Ripple saturation (DN40 3.7): below 150 ms the 50/60 Hz ripple of
	// mains-powered light eats into the usable range.
	if atimeMs < 150 {
		saturation -= saturation / 4
	}
	if float64(c) >= saturation {
		return 0, 0, false
	}

	red, green, blue, clear := float64(r), float64(g), float64(b), float64(c)

	// IR rejection (DN40 3.1): the excess of R+G+B over the clear channel
	// is infrared leaking into all three colour channels.
	infrared := 0.0
	if red+green+blue > clear {
		infrared = (red + green + blue - clear) / 2
	}
	r2, g2, b2 := red-infrared, green-infrared, blue-infrared

	// Lux (DN40 3.2).
	g1 := dn40RCoef*r2 + dn40GCoef*g2 + dn40BCoef*b2
	cpl := (atimeMs * again) / (dn40GlassAttenuation * dn40DeviceFactor)
	if cpl == 0 {
		cpl = 0.001
	}
	lux = g1 / cpl

	// Colour temperature (DN40 3.4).
	if r2 == 0 {
		r2 = 0.001
	}
	return lux, dn40CTCoef*b2/r2 + dn40CTOffset, true
}

// -- Hardware -------------------------------------------------------------

type TCS34725 struct {
	dev *common.I2CDevice
	// The integration time and gain actually programmed, kept here so the
	// DN40 maths needs no extra register reads per measurement.
	integrationMs float64
	gain          float64
}

// NewTCS34725 opens the sensor on /dev/i2c-<bus> at 0x29, verifies the
// sensor ID and programs the same profile as the Python node: 154 ms
// integration at 4x gain, with the ADC left enabled.
//
// The library's own defaults (2.4 ms, 1x) collect almost no light — every
// channel reads 0 in normal room light and the readings collapse to
// constants — which is why both nodes override them.
func NewTCS34725(bus int) (*TCS34725, error) {
	dev, err := common.OpenI2C(bus, 0x29)
	if err != nil {
		return nil, err
	}
	s := &TCS34725{dev: dev}
	sensorID, err := s.readU8(tcsRegSensorID)
	if err != nil {
		return nil, err
	}
	// The library accepts all three IDs the TCS3472 family reports.
	if sensorID != 0x44 && sensorID != 0x10 && sensorID != 0x4D {
		return nil, fmt.Errorf("no TCS34725 at 0x29 (sensor ID 0x%02x, expected 0x44, 0x10 or 0x4d)",
			sensorID)
	}
	if err := s.setIntegrationTime(154); err != nil {
		return nil, err
	}
	if err := s.setGain(4); err != nil {
		return nil, err
	}
	return s, s.activate()
}

func (s *TCS34725) Close() error { return s.dev.Close() }

func (s *TCS34725) readU8(reg uint8) (uint8, error) {
	return s.dev.ReadReg(tcsCommandBit | reg)
}

func (s *TCS34725) writeU8(reg, value uint8) error {
	return s.dev.WriteReg(tcsCommandBit|reg, value)
}

// setIntegrationTime programs the integration time in milliseconds. The
// chip counts 2.4 ms cycles and ATIME holds 256 - cycles.
func (s *TCS34725) setIntegrationTime(milliseconds float64) error {
	if milliseconds < 2.4 || milliseconds > 614.4 {
		return fmt.Errorf("integration time must be between 2.4 and 614.4 ms, got %v", milliseconds)
	}
	cycles := int(milliseconds / 2.4)
	s.integrationMs = float64(cycles) * 2.4
	return s.writeU8(tcsRegATime, uint8(256-cycles))
}

// setGain programs the analog gain (1, 4, 16 or 60); the register holds
// the index into the gain table.
func (s *TCS34725) setGain(gain float64) error {
	for index, value := range tcsGains {
		if value == gain {
			s.gain = gain
			return s.writeU8(tcsRegControl, uint8(index))
		}
	}
	return fmt.Errorf("gain must be one of 1, 4, 16, 60, got %v", gain)
}

// activate powers the chip and enables the ADC, leaving it running — the
// Python node does the same instead of toggling it around every read.
func (s *TCS34725) activate() error {
	enable, err := s.readU8(tcsRegEnable)
	if err != nil {
		return err
	}
	if err := s.writeU8(tcsRegEnable, enable|tcsEnablePON); err != nil {
		return err
	}
	time.Sleep(3 * time.Millisecond) // the oscillator needs 2.4 ms to settle
	return s.writeU8(tcsRegEnable, enable|tcsEnablePON|tcsEnableAEN)
}

// ReadRaw waits for a completed conversion and returns the raw 16-bit red,
// green, blue and clear counts.
func (s *TCS34725) ReadRaw() (r, g, b, c uint16, err error) {
	// One integration period is the longest this can take; give it a few
	// so a conversion that started just before the call still counts.
	deadline := time.Now().Add(time.Duration(3*s.integrationMs+50) * time.Millisecond)
	for {
		status, err := s.readU8(tcsRegStatus)
		if err != nil {
			return 0, 0, 0, 0, err
		}
		if status&tcsStatusAValid != 0 {
			break
		}
		if time.Now().After(deadline) {
			return 0, 0, 0, 0, fmt.Errorf("no completed conversion after %.0f ms", 3*s.integrationMs+50)
		}
		time.Sleep(time.Duration(s.integrationMs+0.9) * time.Millisecond)
	}
	data, err := s.dev.ReadRegs(tcsCommandBit|tcsRegCData, 8)
	if err != nil {
		return 0, 0, 0, 0, err
	}
	word := func(lo int) uint16 { return uint16(data[lo]) | uint16(data[lo+1])<<8 }
	// The block starts at the clear channel, then red, green, blue.
	return word(2), word(4), word(6), word(0), nil
}

// Read returns one measurement as the REST payload, or ok == false when the
// clear channel saturated (no valid colour to report).
//
// Unlike the Python node — which reads the sensor once per property and so
// three times per request — this derives the colour, the illuminance and
// the colour temperature from a single conversion, which also keeps the
// three values consistent with each other.
func (s *TCS34725) Read() (map[string]any, bool, error) {
	r, g, b, c, err := s.ReadRaw()
	if err != nil {
		return nil, false, err
	}
	lux, colorTemperature, ok := temperatureAndLuxDN40(r, g, b, c, s.integrationMs, s.gain)
	if !ok {
		return nil, false, nil
	}
	red, green, blue := colorRGBBytes(r, g, b, c)
	return map[string]any{
		"colorTemperature": int(math.Round(colorTemperature)),
		"lux":              int(math.Round(lux)),
		"red":              red,
		"green":            green,
		"blue":             blue,
	}, true, nil
}
