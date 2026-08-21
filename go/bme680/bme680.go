// Compact BME680 driver, based on the Pimoroni Python driver
// (https://github.com/pimoroni/bme680-python, MIT) with Bosch-datasheet
// corrections. Only what the sensor node needs:
// forced-mode measurements of temperature, pressure, humidity and gas
// resistance with a fixed oversampling/filter/heater profile.
package main

import (
	"sensorplayground/common"

	"fmt"
	"sync"
	"time"
)

const (
	chipIDAddr      = 0xd0
	chipVariantAddr = 0xf0
	chipID          = 0x61
	variantHigh     = 0x01

	softResetAddr = 0xe0
	softResetCmd  = 0xb6

	coeffAddr1    = 0x89
	coeffAddr1Len = 25
	coeffAddr2    = 0xe1
	coeffAddr2Len = 16

	addrResHeatVal   = 0x00
	addrResHeatRange = 0x02
	addrRangeSwErr   = 0x04

	field0Addr  = 0x1d
	fieldLength = 17

	resHeat0Addr = 0x5a
	gasWait0Addr = 0x64

	confOsHAddr          = 0x72
	confTPModeAddr       = 0x74
	confOdrFiltAddr      = 0x75
	confOdrRunGasNbcAddr = 0x71

	newDataMsk  = 0x80
	gasRangeMsk = 0x0f
	heatStabMsk = 0x10
	gasValidMsk = 0x20

	oshMsk, oshPos       = 0x07, 0
	ospMsk, ospPos       = 0x1c, 2
	ostMsk, ostPos       = 0xe0, 5
	filterMsk, filterPos = 0x1c, 2
	runGasMsk, runGasPos = 0x30, 4
	modeMsk, modePos     = 0x03, 0

	sleepMode  = 0
	forcedMode = 1

	osNone, os1x, os2x, os4x, os8x = 0, 1, 2, 3, 4
	filterSize3                    = 2
	enableGasMeasLow               = 0x01
	enableGasMeasHigh              = 0x02

	pollPeriod  = 10 * time.Millisecond
	resetPeriod = 10 * time.Millisecond
)

var lookupTable1 = [16]int64{
	2147483647, 2147483647, 2147483647, 2147483647,
	2147483647, 2126008810, 2147483647, 2130303777, 2147483647,
	2147483647, 2143188679, 2136746228, 2147483647, 2126008810,
	2147483647, 2147483647,
}

var lookupTable2 = [16]int64{
	4096000000, 2048000000, 1024000000, 512000000,
	255744255, 127110228, 64000000, 32258064,
	16016016, 8000000, 4000000, 2000000,
	1000000, 500000, 250000, 125000,
}

type calibration struct {
	parT1, parT2, parT3                             int64
	parP1, parP2, parP3, parP4, parP5               int64
	parP6, parP7, parP8, parP9, parP10              int64
	parH1, parH2, parH3, parH4, parH5, parH6, parH7 int64
	parGH1, parGH2, parGH3                          int64
	tFine                                           int64
	resHeatRange, resHeatVal, rangeSwErr            int64
}

// Readings is one forced-mode measurement.
type Readings struct {
	Temperature   float64 // °C
	Pressure      float64 // hPa
	Humidity      float64 // %RH
	GasResistance float64 // Ohm
	GasValid      bool
	HeatStable    bool
}

type BME680 struct {
	mu      sync.Mutex
	dev     *common.I2CDevice
	calib   calibration
	variant uint8
	ambient int64 // last temperature x100, for the heater calculation
}

// floorDiv mirrors Python's // operator (floor division) for the spots
// where the reference driver relies on it; Go's / truncates toward zero.
func floorDiv(a, b int64) int64 {
	q := a / b
	if (a%b != 0) && ((a < 0) != (b < 0)) {
		q--
	}
	return q
}

func twosComp8(v uint8) int64 { return int64(int8(v)) }

func word(msb, lsb uint8) int64 { return int64(msb)<<8 | int64(lsb) }

func wordSigned(msb, lsb uint8) int64 { return int64(int16(uint16(msb)<<8 | uint16(lsb))) }

// NewBME680 opens the sensor on /dev/i2c-<bus> at 0x76 and applies the
// sensor node's fixed profile: humidity 2x, pressure 4x, temperature 8x,
// IIR filter 3, gas heater 320 °C for 150 ms.
func NewBME680(bus int) (*BME680, error) {
	dev, err := common.OpenI2C(bus, 0x76)
	if err != nil {
		return nil, err
	}
	s := &BME680{dev: dev}

	id, err := dev.ReadReg(chipIDAddr)
	if err != nil {
		dev.Close()
		return nil, fmt.Errorf("reading chip id: %w", err)
	}
	if id != chipID {
		dev.Close()
		return nil, fmt.Errorf("BME680 not found, invalid chip id 0x%02x", id)
	}
	if s.variant, err = dev.ReadReg(chipVariantAddr); err != nil {
		dev.Close()
		return nil, err
	}

	if err := dev.WriteReg(softResetAddr, softResetCmd); err != nil {
		dev.Close()
		return nil, err
	}
	time.Sleep(resetPeriod)

	if err := s.readCalibration(); err != nil {
		dev.Close()
		return nil, err
	}

	runGas := uint8(enableGasMeasLow)
	if s.variant == variantHigh {
		runGas = enableGasMeasHigh
	}
	steps := []struct {
		reg, msk, pos, val uint8
	}{
		{confOsHAddr, oshMsk, oshPos, os2x},    // humidity oversample
		{confTPModeAddr, ospMsk, ospPos, os4x}, // pressure oversample
		{confTPModeAddr, ostMsk, ostPos, os8x}, // temperature oversample
		{confOdrFiltAddr, filterMsk, filterPos, filterSize3},
		{confOdrRunGasNbcAddr, runGasMsk, runGasPos, runGas},
	}
	for _, st := range steps {
		if err := s.setBits(st.reg, st.msk, st.pos, st.val); err != nil {
			dev.Close()
			return nil, err
		}
	}

	// One initial measurement to seed the ambient temperature used by the
	// heater-resistance formula (the Pimoroni constructor does the same).
	if s.Read() == nil {
		dev.Close()
		return nil, fmt.Errorf("initial BME680 measurement timed out")
	}

	if err := dev.WriteReg(resHeat0Addr, s.calcHeaterResistance(320)); err != nil {
		dev.Close()
		return nil, err
	}
	if err := dev.WriteReg(gasWait0Addr, calcHeaterDuration(150)); err != nil {
		dev.Close()
		return nil, err
	}
	return s, nil
}

func (s *BME680) Close() error {
	s.mu.Lock()
	defer s.mu.Unlock()
	return s.dev.Close()
}

func (s *BME680) readCalibration() error {
	c1, err := s.dev.ReadRegs(coeffAddr1, coeffAddr1Len)
	if err != nil {
		return err
	}
	c2, err := s.dev.ReadRegs(coeffAddr2, coeffAddr2Len)
	if err != nil {
		return err
	}
	cal := append(c1, c2...)

	heatRange, err := s.dev.ReadReg(addrResHeatRange)
	if err != nil {
		return err
	}
	heatVal, err := s.dev.ReadReg(addrResHeatVal)
	if err != nil {
		return err
	}
	swErr, err := s.dev.ReadReg(addrRangeSwErr)
	if err != nil {
		return err
	}
	s.calib = parseCalibration(cal, heatRange, heatVal, swErr)
	return nil
}

// parseCalibration decodes the raw calibration blocks; array indices as in
// the Pimoroni driver's set_from_array().
func parseCalibration(cal []byte, heatRange, heatVal, swErr uint8) calibration {
	var cc calibration
	c := &cc
	c.parT1 = word(cal[34], cal[33])
	c.parT2 = wordSigned(cal[2], cal[1])
	c.parT3 = twosComp8(cal[3])
	c.parP1 = word(cal[6], cal[5])
	c.parP2 = wordSigned(cal[8], cal[7])
	c.parP3 = twosComp8(cal[9])
	c.parP4 = wordSigned(cal[12], cal[11])
	c.parP5 = wordSigned(cal[14], cal[13])
	c.parP6 = twosComp8(cal[16])
	c.parP7 = twosComp8(cal[15])
	c.parP8 = wordSigned(cal[20], cal[19])
	c.parP9 = wordSigned(cal[22], cal[21])
	c.parP10 = int64(cal[23])
	c.parH1 = int64(cal[27])<<4 | int64(cal[26]&0x0f)
	c.parH2 = int64(cal[25])<<4 | int64(cal[26]>>4)
	c.parH3 = twosComp8(cal[28])
	c.parH4 = twosComp8(cal[29])
	c.parH5 = twosComp8(cal[30])
	c.parH6 = int64(cal[31])
	c.parH7 = twosComp8(cal[32])
	c.parGH1 = twosComp8(cal[37])
	c.parGH2 = wordSigned(cal[36], cal[35])
	c.parGH3 = twosComp8(cal[38])
	c.resHeatRange = int64(heatRange&0x30) / 16
	c.resHeatVal = twosComp8(heatVal)
	// Register 0x04<7:4> is a signed four-bit value. Sign-extend it before
	// scaling; masking a signed byte first incorrectly turns -4 into 12.
	c.rangeSwErr = int64(int8(swErr)) >> 4
	return cc
}

func (s *BME680) setBits(reg, mask, pos, val uint8) error {
	cur, err := s.dev.ReadReg(reg)
	if err != nil {
		return err
	}
	cur = (cur &^ mask) | (val << pos)
	return s.dev.WriteReg(reg, cur)
}

// Read triggers a forced-mode measurement and returns the readings, or
// nil if no new data arrived within the polling window (like the Python
// driver's get_sensor_data() returning False).
func (s *BME680) Read() *Readings {
	s.mu.Lock()
	defer s.mu.Unlock()

	// A set NEW_DATA bit can still describe the preceding forced sample.
	// Capture its measurement index and wait for the index to advance.
	previousIndex, err := s.dev.ReadReg(field0Addr + 1)
	if err != nil {
		return nil
	}
	if err := s.setBits(confTPModeAddr, modeMsk, modePos, forcedMode); err != nil {
		return nil
	}

	// 30 x 10 ms: a fresh measurement takes ~190 ms with the 150 ms gas
	// heater, so the Pimoroni driver's 10-attempt window only ever caught
	// the previous cycle's data and failed outright on the first read.
	for attempt := 0; attempt < 30; attempt++ {
		status, err := s.dev.ReadReg(field0Addr)
		if err != nil {
			return nil
		}
		if status&newDataMsk == 0 {
			time.Sleep(pollPeriod)
			continue
		}

		regs, err := s.dev.ReadRegs(field0Addr, fieldLength)
		if err != nil {
			return nil
		}
		if regs[1] == previousIndex {
			time.Sleep(pollPeriod)
			continue
		}

		adcPres := int64(regs[2])<<12 | int64(regs[3])<<4 | int64(regs[4])>>4
		adcTemp := int64(regs[5])<<12 | int64(regs[6])<<4 | int64(regs[7])>>4
		adcHum := int64(regs[8])<<8 | int64(regs[9])
		adcGasResLow := int64(regs[13])<<2 | int64(regs[14])>>6
		adcGasResHigh := int64(regs[15])<<2 | int64(regs[16])>>6
		gasRangeL := regs[14] & gasRangeMsk
		gasRangeH := regs[16] & gasRangeMsk

		heatStable := false
		gasValid := false
		if s.variant == variantHigh {
			heatStable = regs[16]&heatStabMsk > 0
			gasValid = regs[16]&gasValidMsk > 0
		} else {
			heatStable = regs[14]&heatStabMsk > 0
			gasValid = regs[14]&gasValidMsk > 0
		}

		tempX100 := s.calcTemperature(adcTemp)
		s.ambient = tempX100

		var gasRes float64
		if s.variant == variantHigh {
			gasRes = calcGasResistanceHigh(adcGasResHigh, gasRangeH)
		} else {
			gasRes = s.calcGasResistanceLow(adcGasResLow, gasRangeL)
		}

		return &Readings{
			Temperature:   float64(tempX100) / 100.0,
			Pressure:      float64(s.calcPressure(adcPres)) / 100.0,
			Humidity:      float64(s.calcHumidity(adcHum)) / 1000.0,
			GasResistance: gasRes,
			GasValid:      gasValid,
			HeatStable:    heatStable,
		}
	}
	return nil
}

func (s *BME680) calcTemperature(adc int64) int64 {
	c := &s.calib
	var1 := (adc >> 3) - (c.parT1 << 1)
	var2 := (var1 * c.parT2) >> 11
	var3 := ((var1 >> 1) * (var1 >> 1)) >> 12
	var3 = (var3 * (c.parT3 << 4)) >> 14
	c.tFine = var2 + var3
	return ((c.tFine * 5) + 128) >> 8
}

func (s *BME680) calcPressure(adc int64) int64 {
	c := &s.calib
	var1 := (c.tFine >> 1) - 64000
	var2 := ((((var1 >> 2) * (var1 >> 2)) >> 11) * c.parP6) >> 2
	var2 = var2 + ((var1 * c.parP5) << 1)
	var2 = (var2 >> 2) + (c.parP4 << 16)
	var1 = ((((var1>>2)*(var1>>2))>>13)*(c.parP3<<5))>>3 + ((c.parP2 * var1) >> 1)
	var1 = var1 >> 18
	var1 = ((32768 + var1) * c.parP1) >> 15

	press := int64(1048576) - adc
	press = (press - (var2 >> 12)) * 3125
	if press >= 1<<31 {
		press = floorDiv(press, var1) << 1
	} else {
		press = floorDiv(press<<1, var1)
	}

	var1 = (c.parP9 * (((press >> 3) * (press >> 3)) >> 13)) >> 12
	var2 = ((press >> 2) * c.parP8) >> 13
	var3 := ((press >> 8) * (press >> 8) * (press >> 8) * c.parP10) >> 17
	return press + ((var1 + var2 + var3 + (c.parP7 << 7)) >> 4)
}

func (s *BME680) calcHumidity(adc int64) int64 {
	c := &s.calib
	tempScaled := ((c.tFine * 5) + 128) >> 8
	var1 := (adc - (c.parH1 * 16)) - (floorDiv(tempScaled*c.parH3, 100) >> 1)
	var2 := (c.parH2 * (floorDiv(tempScaled*c.parH4, 100) +
		floorDiv((tempScaled*floorDiv(tempScaled*c.parH5, 100))>>6, 100) + 16384)) >> 10
	var3 := var1 * var2
	var4 := (c.parH6<<7 + floorDiv(tempScaled*c.parH7, 100)) >> 4
	var5 := ((var3 >> 14) * (var3 >> 14)) >> 10
	var6 := (var4 * var5) >> 1
	hum := (((var3 + var6) >> 10) * 1000) >> 12
	if hum < 0 {
		hum = 0
	}
	if hum > 100000 {
		hum = 100000
	}
	return hum
}

func (s *BME680) calcGasResistanceLow(adc int64, gasRange uint8) float64 {
	c := &s.calib
	var1 := ((1340 + 5*c.rangeSwErr) * lookupTable1[gasRange]) >> 16
	var2 := (adc << 15) - 16777216 + var1
	var3 := (lookupTable2[gasRange] * var1) >> 9
	res := float64(var3+(var2>>1)) / float64(var2)
	if res < 0 {
		res = float64(int64(1)<<32) + res
	}
	return res
}

func calcGasResistanceHigh(adc int64, gasRange uint8) float64 {
	var1 := int64(262144) >> gasRange
	var2 := adc - 512
	var2 *= 3
	var2 = 4096 + var2
	return 10000.0 * float64(var1) / float64(var2) * 100.0
}

func (s *BME680) calcHeaterResistance(temperature int) uint8 {
	if temperature < 200 {
		temperature = 200
	}
	if temperature > 400 {
		temperature = 400
	}
	c := &s.calib
	var1 := float64(s.ambient) * float64(c.parGH3) / 1000.0 * 256.0
	var2 := float64(c.parGH1+784) * ((float64(c.parGH2+154009)*float64(temperature)*5.0/100.0 + 3276800.0) / 10.0)
	var3 := var1 + var2/2.0
	var4 := var3 / float64(c.resHeatRange+4)
	var5 := 131.0*float64(c.resHeatVal) + 65536.0
	heatrResX100 := (var4/var5 - 250.0) * 34.0
	return uint8((heatrResX100 + 50.0) / 100.0)
}

func calcHeaterDuration(durationMs float64) uint8 {
	if durationMs >= 0xfc0 {
		return 0xff
	}
	factor := 0
	for durationMs > 0x3f {
		durationMs /= 4
		factor++
	}
	return uint8(durationMs + float64(factor*64))
}
