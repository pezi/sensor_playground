// Compact VL53L0X driver, ported from the Adafruit CircuitPython driver
// adafruit_vl53l0x (https://github.com/adafruit/Adafruit_CircuitPython_VL53L0X,
// MIT) — the library the Python node uses, itself adapted from the Pololu
// vl53l0x-arduino code: the full init register sequence (reference SPAD
// selection, tuning settings, interrupt config, timing-budget preservation,
// VHV/phase ref calibration), then single-shot ranging reads in millimeters.
// The timeout encoding and timing-budget math are platform-neutral (and
// unit tested); only the I2C access is Linux-only.
//
// Like the Python node (which leaves the driver's io_timeout_s at 0), the
// wait loops poll without a timeout — an unresponsive sensor surfaces as an
// I2C error, not a hang, because every poll is a bus transaction.
package main

import (
	"errors"
	"fmt"
	"math"
	"os"

	"golang.org/x/sys/unix"
)

const (
	vl53l0xAddr = 0x29 // the fixed power-on address (41)

	// Registers (the subset the driver uses, names from the Adafruit driver).
	vlSysrangeStart                        = 0x00
	vlSystemSequenceConfig                 = 0x01
	vlSystemInterruptConfigGpio            = 0x0A
	vlSystemInterruptClear                 = 0x0B
	vlResultInterruptStatus                = 0x13
	vlResultRangeStatus                    = 0x14
	vlFinalRangeConfigMinCountRateRtnLimit = 0x44
	vlMsrcConfigTimeoutMacrop              = 0x46
	vlDynamicSpadNumRequestedRefSpad       = 0x4E
	vlDynamicSpadRefEnStartOffset          = 0x4F
	vlPreRangeConfigVcselPeriod            = 0x50
	vlPreRangeConfigTimeoutMacropHi        = 0x51
	vlMsrcConfigControl                    = 0x60
	vlFinalRangeConfigVcselPeriod          = 0x70
	vlFinalRangeConfigTimeoutMacropHi      = 0x71
	vlGpioHvMuxActiveHigh                  = 0x84
	vlGlobalConfigSpadEnablesRef0          = 0xB0
	vlGlobalConfigRefEnStartSelect         = 0xB6

	i2cSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE
)

// -- Platform-neutral math (mirrors the Adafruit driver's helpers) -----------

// vl53l0xDecodeTimeout decodes a timeout register value; format:
// "(LSByte * 2^MSByte) + 1".
func vl53l0xDecodeTimeout(val uint16) float64 {
	return float64(val&0xFF)*math.Pow(2.0, float64((val&0xFF00)>>8)) + 1
}

// vl53l0xEncodeTimeout encodes a timeout in MCLKs into the register
// format "(LSByte * 2^MSByte) + 1".
func vl53l0xEncodeTimeout(timeoutMclks float64) uint16 {
	mclks := uint32(timeoutMclks) & 0xFFFF
	lsByte := uint32(0)
	msByte := uint32(0)
	if mclks > 0 {
		lsByte = mclks - 1
		for lsByte > 255 {
			lsByte >>= 1
			msByte++
		}
		return uint16((msByte<<8 | lsByte&0xFF) & 0xFFFF)
	}
	return 0
}

// vl53l0xTimeoutMclksToUs converts a timeout in macro clocks to
// microseconds for the given VCSEL period (integer-floor arithmetic like
// the reference driver).
func vl53l0xTimeoutMclksToUs(timeoutPeriodMclks float64, vcselPeriodPclks int) float64 {
	macroPeriodNs := (2304*vcselPeriodPclks*1655 + 500) / 1000
	return math.Floor((timeoutPeriodMclks*float64(macroPeriodNs) + float64(macroPeriodNs/2)) / 1000)
}

// vl53l0xTimeoutUsToMclks converts a timeout in microseconds to macro
// clocks for the given VCSEL period.
func vl53l0xTimeoutUsToMclks(timeoutPeriodUs float64, vcselPeriodPclks int) float64 {
	macroPeriodNs := (2304*vcselPeriodPclks*1655 + 500) / 1000
	return math.Floor((timeoutPeriodUs*1000 + float64(macroPeriodNs/2)) / float64(macroPeriodNs))
}

// vl53l0xDecodeVcselPeriod decodes a VCSEL period register value into PCLKs.
func vl53l0xDecodeVcselPeriod(regVal uint8) int {
	return int((uint16(regVal)+1)&0xFF) << 1
}

type vl53l0xSequenceEnables struct {
	tcc, dss, msrc, preRange, finalRange bool
}

// vl53l0xSequenceStepEnables decodes SYSTEM_SEQUENCE_CONFIG (based on
// VL53L0X_GetSequenceStepEnables from the ST API).
func vl53l0xSequenceStepEnables(sequenceConfig uint8) vl53l0xSequenceEnables {
	return vl53l0xSequenceEnables{
		tcc:        sequenceConfig>>4&0x1 > 0,
		dss:        sequenceConfig>>3&0x1 > 0,
		msrc:       sequenceConfig>>2&0x1 > 0,
		preRange:   sequenceConfig>>6&0x1 > 0,
		finalRange: sequenceConfig>>7&0x1 > 0,
	}
}

// vl53l0xMaskRefSpadMap clears every SPAD enable bit outside the good
// reference window (the first 12 bits are aperture SPADs) and past
// spadCount enabled ones, returning how many stayed enabled.
func vl53l0xMaskRefSpadMap(refSpadMap *[6]byte, spadCount int, spadIsAperture bool) int {
	firstSpadToEnable := 0
	if spadIsAperture {
		firstSpadToEnable = 12
	}
	spadsEnabled := 0
	for i := 0; i < 48; i++ {
		if i < firstSpadToEnable || spadsEnabled == spadCount {
			// This bit is lower than the first one that should be enabled,
			// or (reference_spad_count) bits have already been enabled, so
			// zero this bit.
			refSpadMap[i/8] &^= 1 << (i % 8)
		} else if refSpadMap[i/8]>>(i%8)&0x1 > 0 {
			spadsEnabled++
		}
	}
	return spadsEnabled
}

// vl53l0xTuning is the block of undocumented "default tuning settings"
// every VL53L0X driver writes after the SPAD map (ST API load_tuning_settings).
var vl53l0xTuning = [][2]uint8{
	{0xFF, 0x01}, {0x00, 0x00}, {0xFF, 0x00}, {0x09, 0x00}, {0x10, 0x00},
	{0x11, 0x00}, {0x24, 0x01}, {0x25, 0xFF}, {0x75, 0x00}, {0xFF, 0x01},
	{0x4E, 0x2C}, {0x48, 0x00}, {0x30, 0x20}, {0xFF, 0x00}, {0x30, 0x09},
	{0x54, 0x00}, {0x31, 0x04}, {0x32, 0x03}, {0x40, 0x83}, {0x46, 0x25},
	{0x60, 0x00}, {0x27, 0x00}, {0x50, 0x06}, {0x51, 0x00}, {0x52, 0x96},
	{0x56, 0x08}, {0x57, 0x30}, {0x61, 0x00}, {0x62, 0x00}, {0x64, 0x00},
	{0x65, 0x00}, {0x66, 0xA0}, {0xFF, 0x01}, {0x22, 0x32}, {0x47, 0x14},
	{0x49, 0xFF}, {0x4A, 0x00}, {0xFF, 0x00}, {0x7A, 0x0A}, {0x7B, 0x00},
	{0x78, 0x21}, {0xFF, 0x01}, {0x23, 0x34}, {0x42, 0x00}, {0x44, 0xFF},
	{0x45, 0x26}, {0x46, 0x05}, {0x40, 0x40}, {0x0E, 0x06}, {0x20, 0x1A},
	{0x43, 0x40}, {0xFF, 0x00}, {0x34, 0x03}, {0x35, 0x44}, {0xFF, 0x01},
	{0x31, 0x04}, {0x4B, 0x09}, {0x4C, 0x05}, {0x4D, 0x04}, {0xFF, 0x00},
	{0x44, 0x00}, {0x45, 0x20}, {0x47, 0x08}, {0x48, 0x28}, {0x67, 0x00},
	{0x70, 0x04}, {0x71, 0x01}, {0x72, 0xFE}, {0x76, 0x00}, {0x77, 0x00},
	{0xFF, 0x01}, {0x0D, 0x01}, {0xFF, 0x00}, {0x80, 0x01}, {0x01, 0xF8},
	{0xFF, 0x01}, {0x8E, 0x01}, {0x00, 0x01}, {0xFF, 0x00}, {0x80, 0x00},
}

// -- Device ------------------------------------------------------------------

// VL53L0X reads the distance in millimeters from a VL53L0X.
type VL53L0X struct {
	f                         *os.File
	stopVariable              uint8
	measurementTimingBudgetUs float64
}

// NewVL53L0X opens the sensor on /dev/i2c-<bus> at 0x29 and runs the full
// init sequence, like the Python node's driver.
func NewVL53L0X(bus int) (*VL53L0X, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, vl53l0xAddr); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", vl53l0xAddr, err)
	}
	s := &VL53L0X{f: f}
	if err := s.init(); err != nil {
		f.Close()
		return nil, err
	}
	return s, nil
}

func (s *VL53L0X) Close() error { return s.f.Close() }

func (s *VL53L0X) readU8(reg uint8) (uint8, error) {
	if _, err := s.f.Write([]byte{reg}); err != nil {
		return 0, err
	}
	buf := make([]byte, 1)
	if _, err := s.f.Read(buf); err != nil {
		return 0, err
	}
	return buf[0], nil
}

func (s *VL53L0X) readU16(reg uint8) (uint16, error) {
	if _, err := s.f.Write([]byte{reg}); err != nil {
		return 0, err
	}
	buf := make([]byte, 2)
	if _, err := s.f.Read(buf); err != nil {
		return 0, err
	}
	return uint16(buf[0])<<8 | uint16(buf[1]), nil // big-endian
}

func (s *VL53L0X) writeU8(reg, val uint8) error {
	_, err := s.f.Write([]byte{reg, val})
	return err
}

func (s *VL53L0X) writeU16(reg uint8, val uint16) error {
	_, err := s.f.Write([]byte{reg, byte(val >> 8), byte(val)}) // big-endian
	return err
}

func (s *VL53L0X) writePairs(pairs [][2]uint8) error {
	for _, pair := range pairs {
		if err := s.writeU8(pair[0], pair[1]); err != nil {
			return err
		}
	}
	return nil
}

// init mirrors the Adafruit driver's __init__ (data init, static init,
// SPAD management, tuning settings, interrupt config, timing budget,
// ref calibration).
func (s *VL53L0X) init() error {
	// Check identification registers for expected values (datasheet 3.2).
	for _, id := range []struct{ reg, want uint8 }{{0xC0, 0xEE}, {0xC1, 0xAA}, {0xC2, 0x10}} {
		got, err := s.readU8(id.reg)
		if err != nil {
			return err
		}
		if got != id.want {
			return errors.New("failed to find expected ID register values, check wiring")
		}
	}
	// Set I2C standard mode.
	if err := s.writePairs([][2]uint8{{0x88, 0x00}, {0x80, 0x01}, {0xFF, 0x01}, {0x00, 0x00}}); err != nil {
		return err
	}
	stopVariable, err := s.readU8(0x91)
	if err != nil {
		return err
	}
	s.stopVariable = stopVariable
	if err := s.writePairs([][2]uint8{{0x00, 0x01}, {0xFF, 0x00}, {0x80, 0x00}}); err != nil {
		return err
	}
	// Disable SIGNAL_RATE_MSRC (bit 1) and SIGNAL_RATE_PRE_RANGE (bit 4)
	// limit checks.
	configControl, err := s.readU8(vlMsrcConfigControl)
	if err != nil {
		return err
	}
	if err := s.writeU8(vlMsrcConfigControl, configControl|0x12); err != nil {
		return err
	}
	// Set final range signal rate limit to 0.25 MCPS (million counts per
	// second), as 16-bit 9.7 fixed point.
	if err := s.writeU16(vlFinalRangeConfigMinCountRateRtnLimit, uint16(0.25*(1<<7))); err != nil {
		return err
	}
	if err := s.writeU8(vlSystemSequenceConfig, 0xFF); err != nil {
		return err
	}

	spadCount, spadIsAperture, err := s.getSpadInfo()
	if err != nil {
		return err
	}
	// The SPAD map (RefGoodSpadMap) is read by VL53L0X_get_info_from_device()
	// in the API, but the same data is more easily readable from
	// GLOBAL_CONFIG_SPAD_ENABLES_REF_0 through _6, so read it from there.
	if _, err := s.f.Write([]byte{vlGlobalConfigSpadEnablesRef0}); err != nil {
		return err
	}
	var refSpadMap [6]byte
	if _, err := s.f.Read(refSpadMap[:]); err != nil {
		return err
	}
	if err := s.writePairs([][2]uint8{
		{0xFF, 0x01},
		{vlDynamicSpadRefEnStartOffset, 0x00},
		{vlDynamicSpadNumRequestedRefSpad, 0x2C},
		{0xFF, 0x00},
		{vlGlobalConfigRefEnStartSelect, 0xB4},
	}); err != nil {
		return err
	}
	vl53l0xMaskRefSpadMap(&refSpadMap, spadCount, spadIsAperture)
	if _, err := s.f.Write(append([]byte{vlGlobalConfigSpadEnablesRef0}, refSpadMap[:]...)); err != nil {
		return err
	}

	if err := s.writePairs(vl53l0xTuning); err != nil {
		return err
	}
	if err := s.writeU8(vlSystemInterruptConfigGpio, 0x04); err != nil {
		return err
	}
	gpioHvMux, err := s.readU8(vlGpioHvMuxActiveHigh)
	if err != nil {
		return err
	}
	if err := s.writeU8(vlGpioHvMuxActiveHigh, gpioHvMux&^0x10); err != nil { // active low
		return err
	}
	if err := s.writeU8(vlSystemInterruptClear, 0x01); err != nil {
		return err
	}

	budgetUs, err := s.getMeasurementTimingBudget()
	if err != nil {
		return err
	}
	if err := s.writeU8(vlSystemSequenceConfig, 0xE8); err != nil {
		return err
	}
	if err := s.setMeasurementTimingBudget(budgetUs); err != nil {
		return err
	}
	if err := s.writeU8(vlSystemSequenceConfig, 0x01); err != nil {
		return err
	}
	if err := s.performSingleRefCalibration(0x40); err != nil {
		return err
	}
	if err := s.writeU8(vlSystemSequenceConfig, 0x02); err != nil {
		return err
	}
	if err := s.performSingleRefCalibration(0x00); err != nil {
		return err
	}
	// "restore the previous Sequence Config"
	return s.writeU8(vlSystemSequenceConfig, 0xE8)
}

// getSpadInfo returns the reference SPAD count and type (is_aperture),
// based on the Pololu vl53l0x-arduino code.
func (s *VL53L0X) getSpadInfo() (count int, isAperture bool, err error) {
	if err := s.writePairs([][2]uint8{{0x80, 0x01}, {0xFF, 0x01}, {0x00, 0x00}, {0xFF, 0x06}}); err != nil {
		return 0, false, err
	}
	v, err := s.readU8(0x83)
	if err != nil {
		return 0, false, err
	}
	if err := s.writeU8(0x83, v|0x04); err != nil {
		return 0, false, err
	}
	if err := s.writePairs([][2]uint8{
		{0xFF, 0x07}, {0x81, 0x01}, {0x80, 0x01}, {0x94, 0x6B}, {0x83, 0x00},
	}); err != nil {
		return 0, false, err
	}
	for {
		v, err := s.readU8(0x83)
		if err != nil {
			return 0, false, err
		}
		if v != 0x00 {
			break
		}
	}
	if err := s.writeU8(0x83, 0x01); err != nil {
		return 0, false, err
	}
	tmp, err := s.readU8(0x92)
	if err != nil {
		return 0, false, err
	}
	count = int(tmp & 0x7F)
	isAperture = tmp>>7&0x01 == 1
	if err := s.writePairs([][2]uint8{{0x81, 0x00}, {0xFF, 0x06}}); err != nil {
		return 0, false, err
	}
	v, err = s.readU8(0x83)
	if err != nil {
		return 0, false, err
	}
	if err := s.writeU8(0x83, v&^0x04); err != nil {
		return 0, false, err
	}
	if err := s.writePairs([][2]uint8{{0xFF, 0x01}, {0x00, 0x01}, {0xFF, 0x00}, {0x80, 0x00}}); err != nil {
		return 0, false, err
	}
	return count, isAperture, nil
}

// performSingleRefCalibration is based on
// VL53L0X_perform_single_ref_calibration() from the ST API.
func (s *VL53L0X) performSingleRefCalibration(vhvInitByte uint8) error {
	if err := s.writeU8(vlSysrangeStart, 0x01|vhvInitByte); err != nil {
		return err
	}
	for {
		status, err := s.readU8(vlResultInterruptStatus)
		if err != nil {
			return err
		}
		if status&0x07 != 0 {
			break
		}
	}
	if err := s.writeU8(vlSystemInterruptClear, 0x01); err != nil {
		return err
	}
	return s.writeU8(vlSysrangeStart, 0x00)
}

func (s *VL53L0X) getVcselPulsePeriod(reg uint8) (int, error) {
	val, err := s.readU8(reg)
	if err != nil {
		return 0, err
	}
	return vl53l0xDecodeVcselPeriod(val), nil
}

// getSequenceStepTimeouts is based on get_sequence_step_timeout() from the
// ST API, modified like the Pololu code.
func (s *VL53L0X) getSequenceStepTimeouts(preRange bool) (msrcDssTccUs, preRangeUs, finalRangeUs float64, finalRangeVcselPeriodPclks int, preRangeMclks float64, err error) {
	preRangeVcselPeriodPclks, err := s.getVcselPulsePeriod(vlPreRangeConfigVcselPeriod)
	if err != nil {
		return 0, 0, 0, 0, 0, err
	}
	msrcReg, err := s.readU8(vlMsrcConfigTimeoutMacrop)
	if err != nil {
		return 0, 0, 0, 0, 0, err
	}
	msrcDssTccMclks := float64((uint16(msrcReg) + 1) & 0xFF)
	msrcDssTccUs = vl53l0xTimeoutMclksToUs(msrcDssTccMclks, preRangeVcselPeriodPclks)
	preRangeReg, err := s.readU16(vlPreRangeConfigTimeoutMacropHi)
	if err != nil {
		return 0, 0, 0, 0, 0, err
	}
	preRangeMclks = vl53l0xDecodeTimeout(preRangeReg)
	preRangeUs = vl53l0xTimeoutMclksToUs(preRangeMclks, preRangeVcselPeriodPclks)
	finalRangeVcselPeriodPclks, err = s.getVcselPulsePeriod(vlFinalRangeConfigVcselPeriod)
	if err != nil {
		return 0, 0, 0, 0, 0, err
	}
	finalRangeReg, err := s.readU16(vlFinalRangeConfigTimeoutMacropHi)
	if err != nil {
		return 0, 0, 0, 0, 0, err
	}
	finalRangeMclks := vl53l0xDecodeTimeout(finalRangeReg)
	if preRange {
		finalRangeMclks -= preRangeMclks
	}
	finalRangeUs = vl53l0xTimeoutMclksToUs(finalRangeMclks, finalRangeVcselPeriodPclks)
	return msrcDssTccUs, preRangeUs, finalRangeUs, finalRangeVcselPeriodPclks, preRangeMclks, nil
}

// getMeasurementTimingBudget returns the measurement timing budget in
// microseconds.
func (s *VL53L0X) getMeasurementTimingBudget() (float64, error) {
	budgetUs := float64(1910 + 960) // start overhead + end overhead
	sequenceConfig, err := s.readU8(vlSystemSequenceConfig)
	if err != nil {
		return 0, err
	}
	enables := vl53l0xSequenceStepEnables(sequenceConfig)
	msrcDssTccUs, preRangeUs, finalRangeUs, _, _, err := s.getSequenceStepTimeouts(enables.preRange)
	if err != nil {
		return 0, err
	}
	if enables.tcc {
		budgetUs += msrcDssTccUs + 590
	}
	if enables.dss {
		budgetUs += 2 * (msrcDssTccUs + 690)
	} else if enables.msrc {
		budgetUs += msrcDssTccUs + 660
	}
	if enables.preRange {
		budgetUs += preRangeUs + 660
	}
	if enables.finalRange {
		budgetUs += finalRangeUs + 550
	}
	s.measurementTimingBudgetUs = budgetUs
	return budgetUs, nil
}

// setMeasurementTimingBudget applies a measurement timing budget in
// microseconds by giving the final range step whatever time the other
// enabled steps leave over.
func (s *VL53L0X) setMeasurementTimingBudget(budgetUs float64) error {
	if budgetUs < 20000 {
		return fmt.Errorf("timing budget %v us is below the 20000 us minimum", budgetUs)
	}
	usedBudgetUs := float64(1320 + 960) // start (diff from get) + end overhead
	sequenceConfig, err := s.readU8(vlSystemSequenceConfig)
	if err != nil {
		return err
	}
	enables := vl53l0xSequenceStepEnables(sequenceConfig)
	msrcDssTccUs, preRangeUs, _, finalRangeVcselPeriodPclks, preRangeMclks, err := s.getSequenceStepTimeouts(enables.preRange)
	if err != nil {
		return err
	}
	if enables.tcc {
		usedBudgetUs += msrcDssTccUs + 590
	}
	if enables.dss {
		usedBudgetUs += 2 * (msrcDssTccUs + 690)
	} else if enables.msrc {
		usedBudgetUs += msrcDssTccUs + 660
	}
	if enables.preRange {
		usedBudgetUs += preRangeUs + 660
	}
	if enables.finalRange {
		usedBudgetUs += 550
		// "Note that the final range timeout is determined by the timing
		// budget and the sum of all other timeouts within the sequence.
		// If there is no room for the final range timeout, then an error
		// will be set. Otherwise the remaining time will be applied to
		// the final range."
		if usedBudgetUs > budgetUs {
			return errors.New("requested timeout too big")
		}
		finalRangeTimeoutUs := budgetUs - usedBudgetUs
		finalRangeTimeoutMclks := vl53l0xTimeoutUsToMclks(finalRangeTimeoutUs, finalRangeVcselPeriodPclks)
		if enables.preRange {
			finalRangeTimeoutMclks += preRangeMclks
		}
		if err := s.writeU16(vlFinalRangeConfigTimeoutMacropHi, vl53l0xEncodeTimeout(finalRangeTimeoutMclks)); err != nil {
			return err
		}
		s.measurementTimingBudgetUs = budgetUs
	}
	return nil
}

// Range performs a single-shot range measurement and returns the distance
// in millimeters (adapted from readRangeSingleMillimeters in the Pololu
// code; assumes the default linearity corrective gain of 1000 and no
// fractional ranging).
func (s *VL53L0X) Range() (int, error) {
	if err := s.writePairs([][2]uint8{
		{0x80, 0x01},
		{0xFF, 0x01},
		{0x00, 0x00},
		{0x91, s.stopVariable},
		{0x00, 0x01},
		{0xFF, 0x00},
		{0x80, 0x00},
		{vlSysrangeStart, 0x01},
	}); err != nil {
		return 0, err
	}
	for {
		start, err := s.readU8(vlSysrangeStart)
		if err != nil {
			return 0, err
		}
		if start&0x01 == 0 {
			break
		}
	}
	for {
		status, err := s.readU8(vlResultInterruptStatus)
		if err != nil {
			return 0, err
		}
		if status&0x07 != 0 {
			break
		}
	}
	rangeMm, err := s.readU16(vlResultRangeStatus + 10)
	if err != nil {
		return 0, err
	}
	if err := s.writeU8(vlSystemInterruptClear, 0x01); err != nil {
		return 0, err
	}
	return int(rangeMm), nil
}
