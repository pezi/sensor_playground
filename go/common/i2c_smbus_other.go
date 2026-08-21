//go:build !linux

// The SMBus ioctl is a Linux interface; on other platforms only emulation
// mode works. See i2c_smbus_linux.go for what this does and why.
package common

import "errors"

func (d *I2CDevice) ReadWordSMBus(reg uint8) (uint16, error) {
	return 0, errors.New("SMBus word reads require Linux (use \"emulation\": true elsewhere)")
}
