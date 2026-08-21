//go:build linux

// SMBus word reads via the kernel's I2C_SMBUS ioctl — the same call
// smbus2's read_word_data() makes in the Python nodes.
//
// ReadRegs writes the register address and reads the data in two separate
// transfers, with a stop condition in between. Strict SMBus parts such as
// the MLX90615 reject that: they need a *repeated start*. Letting the
// kernel do the SMBus transaction gets the repeated start (the bcm2835
// adapter has no SMBus hardware, so the kernel emulates the protocol with
// a combined I2C message) and keeps the wire behaviour identical to the
// Python node's.
package common

import (
	"fmt"
	"runtime"
	"unsafe"

	"golang.org/x/sys/unix"
)

const (
	i2cSMBus         = 0x0720 // linux/i2c-dev.h I2C_SMBUS
	i2cSMBusRead     = 1
	i2cSMBusWordData = 3
)

// i2cSMBusIoctlData mirrors struct i2c_smbus_ioctl_data (linux/i2c-dev.h).
type i2cSMBusIoctlData struct {
	ReadWrite uint8
	Command   uint8
	Size      uint32
	Data      uintptr // union i2c_smbus_data *
}

// ReadWordSMBus reads a little-endian 16-bit word from a register with a
// repeated start between the register write and the data read.
func (d *I2CDevice) ReadWordSMBus(reg uint8) (uint16, error) {
	// union i2c_smbus_data is 34 bytes (its block member); a word read
	// fills the first two.
	buf := make([]byte, 34)
	args := i2cSMBusIoctlData{
		ReadWrite: i2cSMBusRead,
		Command:   reg,
		Size:      i2cSMBusWordData,
		Data:      uintptr(unsafe.Pointer(&buf[0])),
	}
	_, _, errno := unix.Syscall(unix.SYS_IOCTL, d.f.Fd(), i2cSMBus,
		uintptr(unsafe.Pointer(&args)))
	runtime.KeepAlive(buf)
	if errno != 0 {
		return 0, fmt.Errorf("SMBus word read of register 0x%02x: %w", reg, errno)
	}
	return uint16(buf[0]) | uint16(buf[1])<<8, nil
}
