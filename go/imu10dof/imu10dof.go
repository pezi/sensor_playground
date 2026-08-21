// Drivers for the Grove IMU 10DOF board: the MPU9250 accelerometer +
// AK8963 magnetometer, ported from the mpu9250-jmdev Python driver (the
// library the Python node uses — bypass mode, 8 g / 16-bit full scale,
// 100 Hz continuous magnetometer), and the BMP280 barometer with the
// Bosch datasheet integer compensation, ported from the Python node's
// own BMP280 class — so the readings match the Python node.
package main

import (
	"fmt"
	"math"
	"time"

	"sensorplayground/common"
)

const (
	mpuAddress = 0x68 // MPU9250 (MPU9050_ADDRESS_68)
	akAddress  = 0x0C // AK8963 magnetometer, visible in bypass mode
	bmpAddress = 0x77 // BMP280 barometer

	// MPU9250 registers.
	mpuSmplrtDiv    = 0x19
	mpuConfig       = 0x1A
	mpuGyroConfig   = 0x1B
	mpuAccelConfig  = 0x1C
	mpuAccelConfig2 = 0x1D
	mpuIntPinCfg    = 0x37
	mpuAccelOut     = 0x3B
	mpuUserCtrl     = 0x6A
	mpuPwrMgmt1     = 0x6B

	// AK8963 registers.
	akMagnetOut = 0x03 // HXL..HZH + ST2 (7 bytes)
	akCntl1     = 0x0A
	akAsaX      = 0x10 // factory sensitivity (FuseROM, 3 bytes)

	// Full-scale selections the Python node configures.
	gfs1000      = 0x02 // gyro 1000 dps
	afs8G        = 0x02 // accel 8 g
	akBit16      = 0x01 // magnetometer 16-bit output
	akModeC100Hz = 0x06 // continuous 100 Hz
)

const (
	accelScale = 8.0 / 32768.0    // ACCEL_SCALE_MODIFIER_8G
	magScale   = 4912.0 / 32760.0 // MAGNOMETER_SCALE_MODIFIER_BIT_16
)

// -- MPU9250 (accelerometer + AK8963 magnetometer) ---------------------------

type MPU9250 struct {
	mpu    *common.I2CDevice
	ak     *common.I2CDevice
	magCal [3]float64 // AK8963 factory sensitivity adjustment
}

// NewMPU9250 opens the MPU9250 and its AK8963 on /dev/i2c-<bus> and runs
// the mpu9250-jmdev configuration sequence (retried up to three times,
// like the reference driver).
func NewMPU9250(bus int) (*MPU9250, error) {
	mpu, err := common.OpenI2C(bus, mpuAddress)
	if err != nil {
		return nil, err
	}
	ak, err := common.OpenI2C(bus, akAddress)
	if err != nil {
		mpu.Close()
		return nil, err
	}
	m := &MPU9250{mpu: mpu, ak: ak}
	for attempt := 1; ; attempt++ {
		if err = m.configure(); err == nil {
			break
		}
		if attempt >= 3 {
			m.Close()
			return nil, fmt.Errorf("configuring MPU9250: %w", err)
		}
	}
	return m, nil
}

func (m *MPU9250) Close() {
	m.mpu.Close()
	m.ak.Close()
}

func (m *MPU9250) configure() error {
	// MPU6500 core (configureMPU6500, no slave).
	steps := []struct {
		reg, val uint8
		wait     time.Duration
	}{
		{mpuPwrMgmt1, 0x00, 100 * time.Millisecond},  // sleep off
		{mpuPwrMgmt1, 0x01, 100 * time.Millisecond},  // auto select clock source
		{mpuConfig, 0x00, 0},                         // DLPF_CFG
		{mpuSmplrtDiv, 0x00, 0},                      // sample rate divider
		{mpuGyroConfig, gfs1000 << 3, 0},             // gyro full scale select
		{mpuAccelConfig, afs8G << 3, 0},              // accel full scale select
		{mpuAccelConfig2, 0x00, 0},                   // A_DLPFCFG
		{mpuIntPinCfg, 0x02, 100 * time.Millisecond}, // BYPASS_EN enable
		{mpuUserCtrl, 0x00, 100 * time.Millisecond},  // disable master
	}
	for _, s := range steps {
		if err := m.mpu.WriteReg(s.reg, s.val); err != nil {
			return err
		}
		if s.wait > 0 {
			time.Sleep(s.wait)
		}
	}

	// AK8963 magnetometer (configureAK8963): read the factory sensitivity
	// coefficients from FuseROM, then 16-bit continuous 100 Hz mode.
	writeAK := func(val uint8) error {
		if err := m.ak.WriteReg(akCntl1, val); err != nil {
			return err
		}
		time.Sleep(100 * time.Millisecond)
		return nil
	}
	if err := writeAK(0x00); err != nil { // power down
		return err
	}
	if err := writeAK(0x0F); err != nil { // FuseROM access mode
		return err
	}
	asa, err := m.ak.ReadRegs(akAsaX, 3)
	if err != nil {
		return err
	}
	if err := writeAK(0x00); err != nil { // power down
		return err
	}
	if err := writeAK(akBit16<<4 | akModeC100Hz); err != nil { // scale + continuous mode
		return err
	}
	for i := 0; i < 3; i++ {
		m.magCal[i] = (float64(asa[i])-128)/256.0 + 1.0
	}
	return nil
}

// ReadAccel returns the acceleration in g; a failed I2C read yields
// zeros, like the reference driver's getDataError.
func (m *MPU9250) ReadAccel() (x, y, z float64) {
	data, err := m.mpu.ReadRegs(mpuAccelOut, 6)
	if err != nil {
		return 0, 0, 0
	}
	return convertAccel(data)
}

// ReadMag returns the magnetic field in µT; a failed read yields zeros.
func (m *MPU9250) ReadMag() (x, y, z float64) {
	data, err := m.ak.ReadRegs(akMagnetOut, 7)
	if err != nil {
		return 0, 0, 0
	}
	return convertMag(data, m.magCal)
}

// convertAccel scales a big-endian 6-byte accelerometer block to g at the
// 8 g full scale.
func convertAccel(data []byte) (x, y, z float64) {
	s16 := func(msb, lsb byte) float64 {
		return float64(int16(uint16(msb)<<8 | uint16(lsb)))
	}
	x = s16(data[0], data[1]) * accelScale
	y = s16(data[2], data[3]) * accelScale
	z = s16(data[4], data[5]) * accelScale
	return
}

// convertMag scales a little-endian 7-byte magnetometer block (HXL..ST2)
// to µT at the 16-bit full scale, applying the factory sensitivity. A
// set ST2 overflow bit yields zeros, like the reference driver.
func convertMag(data []byte, magCal [3]float64) (x, y, z float64) {
	if data[6]&0x08 == 0x08 { // magnetic sensor overflow
		return 0, 0, 0
	}
	s16 := func(lsb, msb byte) float64 {
		return float64(int16(uint16(msb)<<8 | uint16(lsb)))
	}
	x = s16(data[0], data[1]) * magScale * magCal[0]
	y = s16(data[2], data[3]) * magScale * magCal[1]
	z = s16(data[4], data[5]) * magScale * magCal[2]
	return
}

// anglesFrom reduces the MPU9250 axes to the node's derived readings:
// roll/pitch from the accelerometer, compass heading from the raw
// magnetometer (not tilt-compensated) and the total acceleration
// magnitude (g-force).
func anglesFrom(ax, ay, az, mx, my float64) (roll, pitch, heading, gforce float64) {
	degrees := func(rad float64) float64 { return rad * 180.0 / math.Pi }
	roll = degrees(math.Atan2(ay, az))
	pitch = degrees(math.Atan2(-ax, math.Sqrt(ay*ay+az*az)))
	heading = math.Mod(degrees(math.Atan2(my, mx)), 360.0)
	if heading < 0 {
		heading += 360.0
	}
	gforce = math.Sqrt(ax*ax + ay*ay + az*az)
	return
}

// -- BMP280 barometer ---------------------------------------------------------

const bmpChipID = 0x58 // value of the id register (0xD0) for the BMP280

type bmp280Calibration struct {
	digT1, digT2, digT3                                           int64
	digP1, digP2, digP3, digP4, digP5, digP6, digP7, digP8, digP9 int64
}

// BMP280 implements the Bosch datasheet algorithm, like the Python
// node's own minimal driver. The Grove IMU 10DOF v2.0 (2016) replaced
// the original BMP180 with a BMP280; the two chips share the I2C address
// but use different registers and a different compensation algorithm.
type BMP280 struct {
	dev   *common.I2CDevice
	calib bmp280Calibration
}

// NewBMP280 opens the barometer on /dev/i2c-<bus> at 0x77, checks the
// chip id and starts normal-mode x1 sampling.
func NewBMP280(bus int) (*BMP280, error) {
	dev, err := common.OpenI2C(bus, bmpAddress)
	if err != nil {
		return nil, err
	}
	chipID, err := dev.ReadReg(0xD0)
	if err != nil {
		dev.Close()
		return nil, err
	}
	if chipID != bmpChipID {
		dev.Close()
		return nil, fmt.Errorf("BMP280 not found at 0x%02X (id register returned 0x%02X)", bmpAddress, chipID)
	}

	calib, err := dev.ReadRegs(0x88, 24)
	if err != nil {
		dev.Close()
		return nil, err
	}
	s := &BMP280{dev: dev, calib: parseBMP280Calibration(calib)}

	// ctrl_meas: temperature x1, pressure x1, normal mode.
	if err := dev.WriteReg(0xF4, 0x27); err != nil {
		dev.Close()
		return nil, err
	}
	// config: 1000 ms standby, filter off.
	if err := dev.WriteReg(0xF5, 0xA0); err != nil {
		dev.Close()
		return nil, err
	}
	time.Sleep(50 * time.Millisecond)
	return s, nil
}

func (s *BMP280) Close() error { return s.dev.Close() }

// parseBMP280Calibration reads the little-endian dig_T*/dig_P* words
// (T1/P1 unsigned, the rest signed).
func parseBMP280Calibration(calib []byte) bmp280Calibration {
	u16 := func(i int) int64 { return int64(calib[i]) | int64(calib[i+1])<<8 }
	s16 := func(i int) int64 {
		v := u16(i)
		if v > 32767 {
			v -= 65536
		}
		return v
	}
	return bmp280Calibration{
		digT1: u16(0), digT2: s16(2), digT3: s16(4),
		digP1: u16(6), digP2: s16(8), digP3: s16(10),
		digP4: s16(12), digP5: s16(14), digP6: s16(16),
		digP7: s16(18), digP8: s16(20), digP9: s16(22),
	}
}

// Read returns (temperature in °C, pressure in hPa).
func (s *BMP280) Read() (temperature, pressure float64, err error) {
	raw, err := s.dev.ReadRegs(0xF7, 6)
	if err != nil {
		return 0, 0, err
	}
	adcP := int64(raw[0])<<12 | int64(raw[1])<<4 | int64(raw[2])>>4
	adcT := int64(raw[3])<<12 | int64(raw[4])<<4 | int64(raw[5])>>4
	temperature, pressure = compensateBMP280(&s.calib, adcT, adcP)
	return temperature, pressure, nil
}

// floorDiv mirrors Python's // operator (the compensation's division
// rounds toward negative infinity, not toward zero).
func floorDiv(a, b int64) int64 {
	q := a / b
	if a%b != 0 && (a < 0) != (b < 0) {
		q--
	}
	return q
}

// compensateBMP280 converts raw ADC values to (°C, hPa) with the Bosch
// datasheet integer algorithms (32-bit temperature, 64-bit pressure),
// exactly as transcribed in the Python node.
func compensateBMP280(c *bmp280Calibration, adcT, adcP int64) (temperature, pressure float64) {
	// Temperature compensation (datasheet 32-bit integer algorithm).
	var1 := (((adcT >> 3) - (c.digT1 << 1)) * c.digT2) >> 11
	var2 := (((((adcT >> 4) - c.digT1) * ((adcT >> 4) - c.digT1)) >> 12) * c.digT3) >> 14
	tFine := var1 + var2
	temperature = float64((tFine*5+128)>>8) / 100.0

	// Pressure compensation (datasheet 64-bit integer algorithm).
	var1 = tFine - 128000
	var2 = var1 * var1 * c.digP6
	var2 += (var1 * c.digP5) << 17
	var2 += c.digP4 << 35
	var1 = ((var1 * var1 * c.digP3) >> 8) + ((var1 * c.digP2) << 12)
	var1 = (((int64(1) << 47) + var1) * c.digP1) >> 33
	if var1 == 0 {
		return temperature, 0.0 // avoid division by zero
	}
	p := int64(1048576) - adcP
	p = floorDiv(((p<<31)-var2)*3125, var1)
	var1 = (c.digP9 * (p >> 13) * (p >> 13)) >> 25
	var2 = (c.digP8 * p) >> 19
	p = ((p + var1 + var2) >> 8) + (c.digP7 << 4)

	// p is in Q24.8 Pa; convert to hPa.
	return temperature, (float64(p) / 256.0) / 100.0
}

// -- Combined sensor ----------------------------------------------------------

// IMU10DOFReadings is one set of derived readings for the live payload.
type IMU10DOFReadings struct {
	Roll, Pitch, Heading, GForce float64
	Temperature, Pressure        float64
}

// IMU10DOF reads roll/pitch/heading/g-force plus temperature and pressure.
type IMU10DOF struct {
	mpu *MPU9250
	bmp *BMP280
}

func NewIMU10DOF(bus int) (*IMU10DOF, error) {
	mpu, err := NewMPU9250(bus)
	if err != nil {
		return nil, err
	}
	bmp, err := NewBMP280(bus)
	if err != nil {
		mpu.Close()
		return nil, err
	}
	return &IMU10DOF{mpu: mpu, bmp: bmp}, nil
}

func (s *IMU10DOF) Read() (*IMU10DOFReadings, error) {
	ax, ay, az := s.mpu.ReadAccel()
	mx, my, _ := s.mpu.ReadMag()
	roll, pitch, heading, gforce := anglesFrom(ax, ay, az, mx, my)
	temperature, pressure, err := s.bmp.Read()
	if err != nil {
		return nil, err
	}
	return &IMU10DOFReadings{
		Roll: roll, Pitch: pitch, Heading: heading, GForce: gforce,
		Temperature: temperature, Pressure: pressure,
	}, nil
}
