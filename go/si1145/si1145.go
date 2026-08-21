// Compact SI1145 driver — a port of the `SI1145` PyPI package the Python
// node uses (itself a port of Adafruit's Arduino library): the same reset
// sequence, the same UV calibration coefficients, the same channel list
// and ADC settings, and the same autonomous measurement mode, so the
// counts this node reports match the Python node's.
//
// The chip computes the UV index itself from the visible/IR photodiodes
// and reports it multiplied by 100; visible and IR are raw counts (the
// SI1145 is not lux-calibrated) that sit at a dark baseline of roughly
// 250-260 rather than 0.
package main

import (
	"fmt"
	"time"

	"sensorplayground/common"
)

// -- SI1145 register map --------------------------------------------------

const (
	siAddress = 0x60

	siRegPartID     = 0x00 // reads 0x45 on an SI1145
	siRegIntCfg     = 0x03
	siRegIrqEn      = 0x04
	siRegIrqMode1   = 0x05
	siRegIrqMode2   = 0x06
	siRegHwKey      = 0x07
	siRegMeasRate0  = 0x08
	siRegMeasRate1  = 0x09
	siRegPsLed21    = 0x0F
	siRegUCoeff0    = 0x13
	siRegUCoeff1    = 0x14
	siRegUCoeff2    = 0x15
	siRegUCoeff3    = 0x16
	siRegParamWr    = 0x17
	siRegCommand    = 0x18
	siRegIrqStat    = 0x21
	siRegAlsVisData = 0x22 // 16-bit little-endian, like the two below
	siRegAlsIrData  = 0x24
	siRegUVIndex    = 0x2C
	siRegParamRd    = 0x2E

	siPartID = 0x45

	// Commands.
	siCmdReset     = 0x01
	siCmdParamSet  = 0xA0
	siCmdPsAlsAuto = 0x0F

	// Parameter RAM addresses.
	siParamChList         = 0x01
	siParamPsLed12Sel     = 0x02
	siParamPs1AdcMux      = 0x07
	siParamPsAdcCounter   = 0x0A
	siParamPsAdcGain      = 0x0B
	siParamPsAdcMisc      = 0x0C
	siParamAlsIrAdcMux    = 0x0E
	siParamAlsVisAdcCount = 0x10
	siParamAlsVisAdcGain  = 0x11
	siParamAlsVisAdcMisc  = 0x12
	siParamAlsIrAdcCount  = 0x1D
	siParamAlsIrAdcGain   = 0x1E
	siParamAlsIrAdcMisc   = 0x1F

	// Parameter values.
	siChListEnUV            = 0x80
	siChListEnAlsIr         = 0x20
	siChListEnAlsVis        = 0x10
	siChListEnPs1           = 0x01
	siIntCfgIntOe           = 0x01
	siIrqEnAlsEverySmpl     = 0x01
	siPsLed12SelPs1Led1     = 0x01
	siAdcCounter511Clk      = 0x70
	siAdcMuxSmallIr         = 0x00
	siAdcMuxLargeIr         = 0x03
	siPsAdcMiscRange        = 0x20
	siPsAdcMiscPsMode       = 0x04
	siAlsVisAdcMiscVisRange = 0x20
	siAlsIrAdcMiscRange     = 0x20
)

// uvIndex converts the chip's raw UV register value to a UV index: the
// SI1145 reports the index multiplied by 100.
func uvIndex(raw uint16) float64 { return float64(raw) / 100.0 }

// -- Hardware -------------------------------------------------------------

type SI1145 struct {
	dev *common.I2CDevice
}

// NewSI1145 opens the sensor on /dev/i2c-<bus> at 0x60, resets it and
// starts the autonomous measurement loop.
func NewSI1145(bus int) (*SI1145, error) {
	dev, err := common.OpenI2C(bus, siAddress)
	if err != nil {
		return nil, err
	}
	s := &SI1145{dev: dev}
	// The Python library does not check the part ID; doing so turns a
	// missing or wrong chip into a clear error instead of nonsense counts.
	partID, err := dev.ReadReg(siRegPartID)
	if err != nil {
		return nil, err
	}
	if partID != siPartID {
		return nil, fmt.Errorf("No SI1145 at 0x%02x (part ID 0x%02x, expected 0x%02x)",
			siAddress, partID, siPartID)
	}
	if err := s.reset(); err != nil {
		return nil, err
	}
	if err := s.loadCalibration(); err != nil {
		return nil, err
	}
	return s, nil
}

func (s *SI1145) Close() error { return s.dev.Close() }

// reset stops any running measurement, clears the interrupt state and
// unlocks the chip with the hardware key from the datasheet.
func (s *SI1145) reset() error {
	writes := []struct{ reg, val uint8 }{
		{siRegMeasRate0, 0x00},
		{siRegMeasRate1, 0x00},
		{siRegIrqEn, 0x00},
		{siRegIrqMode1, 0x00},
		{siRegIrqMode2, 0x00},
		{siRegIntCfg, 0x00},
		{siRegIrqStat, 0xFF},
		{siRegCommand, siCmdReset},
	}
	for _, w := range writes {
		if err := s.dev.WriteReg(w.reg, w.val); err != nil {
			return err
		}
	}
	time.Sleep(10 * time.Millisecond)
	if err := s.dev.WriteReg(siRegHwKey, 0x17); err != nil {
		return err
	}
	time.Sleep(10 * time.Millisecond)
	return nil
}

// writeParam writes one byte of parameter RAM, which is only reachable
// through the command register.
func (s *SI1145) writeParam(param, value uint8) error {
	if err := s.dev.WriteReg(siRegParamWr, value); err != nil {
		return err
	}
	if err := s.dev.WriteReg(siRegCommand, param|siCmdParamSet); err != nil {
		return err
	}
	_, err := s.dev.ReadReg(siRegParamRd) // read back, as the library does
	return err
}

// loadCalibration writes the UV coefficients, enables the UV, visible, IR
// and proximity channels, configures the ADCs for the fastest clock with
// 511-clock measurements, and starts autonomous sampling every 8 ms.
func (s *SI1145) loadCalibration() error {
	// UV index coefficients from the datasheet's default calibration.
	for _, w := range []struct{ reg, val uint8 }{
		{siRegUCoeff0, 0x29},
		{siRegUCoeff1, 0x89},
		{siRegUCoeff2, 0x02},
		{siRegUCoeff3, 0x00},
	} {
		if err := s.dev.WriteReg(w.reg, w.val); err != nil {
			return err
		}
	}
	if err := s.writeParam(siParamChList,
		siChListEnUV|siChListEnAlsIr|siChListEnAlsVis|siChListEnPs1); err != nil {
		return err
	}
	// Interrupt on every ALS sample (wired out on some breakouts; the node
	// polls the data registers regardless).
	if err := s.dev.WriteReg(siRegIntCfg, siIntCfgIntOe); err != nil {
		return err
	}
	if err := s.dev.WriteReg(siRegIrqEn, siIrqEnAlsEverySmpl); err != nil {
		return err
	}
	// Proximity: 20 mA on LED 1, high range, large-IR photodiode.
	if err := s.dev.WriteReg(siRegPsLed21, 0x03); err != nil {
		return err
	}
	params := []struct{ param, value uint8 }{
		{siParamPs1AdcMux, siAdcMuxLargeIr},
		{siParamPsLed12Sel, siPsLed12SelPs1Led1},
		{siParamPsAdcGain, 0},
		{siParamPsAdcCounter, siAdcCounter511Clk},
		{siParamPsAdcMisc, siPsAdcMiscRange | siPsAdcMiscPsMode},
		// Ambient light: small-IR photodiode for IR, both in high range.
		{siParamAlsIrAdcMux, siAdcMuxSmallIr},
		{siParamAlsIrAdcGain, 0},
		{siParamAlsIrAdcCount, siAdcCounter511Clk},
		{siParamAlsIrAdcMisc, siAlsIrAdcMiscRange},
		{siParamAlsVisAdcGain, 0},
		{siParamAlsVisAdcCount, siAdcCounter511Clk},
		{siParamAlsVisAdcMisc, siAlsVisAdcMiscVisRange},
	}
	for _, p := range params {
		if err := s.writeParam(p.param, p.value); err != nil {
			return err
		}
	}
	// 255 * 31.25 µs ≈ 8 ms between autonomous measurements.
	if err := s.dev.WriteReg(siRegMeasRate0, 0xFF); err != nil {
		return err
	}
	return s.dev.WriteReg(siRegCommand, siCmdPsAlsAuto)
}

// readWord reads one of the 16-bit little-endian result registers.
func (s *SI1145) readWord(reg uint8) (uint16, error) {
	data, err := s.dev.ReadRegs(reg, 2)
	if err != nil {
		return 0, err
	}
	return uint16(data[0]) | uint16(data[1])<<8, nil
}

// Read returns the latest visible and IR counts and the UV index.
func (s *SI1145) Read() (visible, ir uint16, uv float64, err error) {
	if visible, err = s.readWord(siRegAlsVisData); err != nil {
		return 0, 0, 0, err
	}
	if ir, err = s.readWord(siRegAlsIrData); err != nil {
		return 0, 0, 0, err
	}
	rawUV, err := s.readWord(siRegUVIndex)
	if err != nil {
		return 0, 0, 0, err
	}
	return visible, ir, uvIndex(rawUV), nil
}
