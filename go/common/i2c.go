// Minimal /dev/i2c-N wrapper — just enough for the BME680 driver.
//
// Uses plain write/read syscalls after an I2C_SLAVE ioctl. The BME680
// tolerates a stop condition between the register-address write and the
// data read, so combined I2C_RDWR transactions are not required.
package common

import (
	"fmt"
	"os"

	"golang.org/x/sys/unix"
)

const i2cSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE

type I2CDevice struct {
	f    *os.File
	addr uint8
}

func OpenI2C(bus int, addr uint8) (*I2CDevice, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, int(addr)); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", addr, err)
	}
	return &I2CDevice{f: f, addr: addr}, nil
}

func (d *I2CDevice) Close() error { return d.f.Close() }

// WriteReg writes a single value to a register.
func (d *I2CDevice) WriteReg(reg, val uint8) error {
	_, err := d.f.Write([]byte{reg, val})
	return err
}

// WriteBytes writes a raw byte sequence in one transfer — for devices like
// the SSD1306 that take a control byte followed by a whole frame rather
// than register/value pairs. The i2c-dev driver caps a single write at
// 8192 bytes, well above the 1025 a full display frame needs.
func (d *I2CDevice) WriteBytes(data []byte) error {
	_, err := d.f.Write(data)
	return err
}

// ReadRegs reads length bytes starting at reg.
func (d *I2CDevice) ReadRegs(reg uint8, length int) ([]byte, error) {
	if _, err := d.f.Write([]byte{reg}); err != nil {
		return nil, err
	}
	buf := make([]byte, length)
	if _, err := d.f.Read(buf); err != nil {
		return nil, err
	}
	return buf, nil
}

// ReadReg reads a single register.
func (d *I2CDevice) ReadReg(reg uint8) (uint8, error) {
	buf, err := d.ReadRegs(reg, 1)
	if err != nil {
		return 0, err
	}
	return buf[0], nil
}
