// M24LR64E-R user-memory access over /dev/i2c-N — the I2C side of the
// Grove NFC Tag's dual-interface EEPROM (16-bit memory addressing).
//
// The common package's I2CDevice speaks 8-bit register maps, so this tag
// gets its own minimal wrapper: an I2C_SLAVE ioctl, then a two-byte
// address write followed by a sequential read per chunk. The Python node
// reads the same chunks in one combined i2c_rdwr transaction; the
// M24LR64's address pointer survives the stop condition in between, so
// separate write/read transactions return the same bytes.
package main

import (
	"fmt"
	"io"
	"os"

	"golang.org/x/sys/unix"
)

const (
	i2cSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE
	i2cChunk = 32     // bytes per I2C transaction, safe on every adapter
)

// Tag is what the content loop polls: the hardware tag or the emulation.
type Tag interface {
	ReadContent() (Content, error)
	Close()
}

// M24lr64Tag reads the M24LR64E-R user memory over I2C.
type M24lr64Tag struct {
	f *os.File
}

// OpenM24lr64 opens the tag and reads it once, failing fast on a
// wiring/address problem (like the Python node's constructor).
func OpenM24lr64(bus int, addr uint8) (*M24lr64Tag, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, int(addr)); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", addr, err)
	}
	tag := &M24lr64Tag{f: f}
	if _, err := tag.ReadContent(); err != nil {
		f.Close()
		return nil, err
	}
	return tag, nil
}

// ReadContent scans the NDEF area and returns the parsed content.
func (t *M24lr64Tag) ReadContent() (Content, error) {
	data := make([]byte, 0, scanLength)
	buf := make([]byte, i2cChunk)
	for offset := 0; offset < scanLength; offset += i2cChunk {
		if _, err := t.f.Write([]byte{byte(offset >> 8), byte(offset & 0xFF)}); err != nil {
			return Content{}, err
		}
		if _, err := io.ReadFull(t.f, buf); err != nil {
			return Content{}, err
		}
		data = append(data, buf...)
	}
	return parseNdefArea(data), nil
}

func (t *M24lr64Tag) Close() { t.f.Close() }
