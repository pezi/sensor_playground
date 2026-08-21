// AT24C128 text-region access over /dev/i2c-N — a 128 Kbit (16 KB) serial
// EEPROM with 16-bit memory addressing.
//
// The common package's I2CDevice speaks 8-bit register maps, so the chip
// gets its own minimal wrapper: an I2C_SLAVE ioctl, then a two-byte memory
// address in front of every transaction. The Python node reads a chunk in
// one combined i2c_rdwr transaction; the AT24C128's address counter
// survives the stop condition in between, so separate write/read
// transactions return the same bytes.
package main

import (
	"bytes"
	"fmt"
	"io"
	"os"
	"strings"
	"time"
	"unicode/utf8"

	"golang.org/x/sys/unix"
)

const (
	i2cSlave = 0x0703 // linux/i2c-dev.h I2C_SLAVE

	// EEPROM text region: 2-byte magic + 2-byte big-endian length + up to
	// 512 bytes of UTF-8 text.
	headerSize   = 4
	textMaxBytes = 512

	// AT24C128 write page; a write transaction must not cross a page
	// boundary or it wraps around inside the page.
	pageSize = 64

	// Bytes per I2C transaction, safe on every adapter.
	ioChunk = 32

	// Worst-case internal write cycle per the datasheet is 5 ms.
	writeCycle = 6 * time.Millisecond
)

// magic marks a text region written by Sensor Playground; a chip without
// it (e.g. factory-fresh, all 0xFF) holds no text.
var magic = []byte{'S', 'P'}

// Eeprom is what the controller stores the text on: the real chip or the
// emulation.
type Eeprom interface {
	ReadText() (string, error)
	WriteText(text []byte) error
	Close()
}

// -- Record encoding ---------------------------------------------------------

// encodeRecord returns the bytes stored at offset 0 for [text]: the magic,
// the big-endian byte length and the UTF-8 text itself.
func encodeRecord(text []byte) []byte {
	record := make([]byte, 0, headerSize+len(text))
	record = append(record, magic...)
	record = append(record, byte(len(text)>>8), byte(len(text)))
	return append(record, text...)
}

// recordLength returns the stored text length read from a header, or -1
// when the chip holds no usable record: a missing magic or an implausible
// length reads as an empty text rather than as garbage.
func recordLength(header []byte) int {
	if len(header) < headerSize || !bytes.Equal(header[:2], magic) {
		return -1
	}
	length := int(header[2])<<8 | int(header[3])
	if length > textMaxBytes {
		return -1
	}
	return length
}

// decodeRecord parses a whole record (header + payload) into the stored
// text — the pure counterpart of At24c128Eeprom.ReadText.
func decodeRecord(record []byte) string {
	length := recordLength(record)
	if length <= 0 {
		return ""
	}
	end := headerSize + length
	if end > len(record) {
		end = len(record) // truncated dump: report what is there
	}
	return decodeUTF8Replace(record[headerSize:end])
}

// decodeUTF8Replace decodes UTF-8 with U+FFFD for each invalid byte, like
// Python's errors="replace".
func decodeUTF8Replace(b []byte) string {
	var sb strings.Builder
	for len(b) > 0 {
		r, size := utf8.DecodeRune(b)
		if r == utf8.RuneError && size == 1 {
			sb.WriteRune(utf8.RuneError)
		} else {
			sb.WriteRune(r)
		}
		b = b[size:]
	}
	return sb.String()
}

// -- Hardware ----------------------------------------------------------------

// At24c128Eeprom reads and writes the AT24C128 text region over I2C.
type At24c128Eeprom struct {
	f *os.File
}

// OpenAt24c128 opens the chip and reads it once, failing fast on a
// wiring/address problem (like the Python node's constructor).
func OpenAt24c128(bus int, addr uint8) (*At24c128Eeprom, error) {
	f, err := os.OpenFile(fmt.Sprintf("/dev/i2c-%d", bus), os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	if err := unix.IoctlSetInt(int(f.Fd()), i2cSlave, int(addr)); err != nil {
		f.Close()
		return nil, fmt.Errorf("I2C_SLAVE ioctl for 0x%02x: %w", addr, err)
	}
	eeprom := &At24c128Eeprom{f: f}
	if _, err := eeprom.ReadText(); err != nil {
		f.Close()
		return nil, err
	}
	return eeprom, nil
}

// read random-reads [length] bytes starting at [addr].
func (e *At24c128Eeprom) read(addr, length int) ([]byte, error) {
	data := make([]byte, length)
	for offset := 0; offset < length; offset += ioChunk {
		at := addr + offset
		count := ioChunk
		if remaining := length - offset; remaining < count {
			count = remaining
		}
		if _, err := e.f.Write([]byte{byte(at >> 8), byte(at)}); err != nil {
			return nil, err
		}
		if _, err := io.ReadFull(e.f, data[offset:offset+count]); err != nil {
			return nil, err
		}
	}
	return data, nil
}

// write writes [data] starting at [addr], splitting the transactions so
// none crosses a 64-byte page boundary, and waiting out the chip's
// internal write cycle after each one.
func (e *At24c128Eeprom) write(addr int, data []byte) error {
	for offset := 0; offset < len(data); {
		at := addr + offset
		count := ioChunk
		if pageLeft := pageSize - at%pageSize; pageLeft < count {
			count = pageLeft
		}
		if remaining := len(data) - offset; remaining < count {
			count = remaining
		}
		frame := make([]byte, 0, 2+count)
		frame = append(frame, byte(at>>8), byte(at))
		frame = append(frame, data[offset:offset+count]...)
		if _, err := e.f.Write(frame); err != nil {
			return err
		}
		time.Sleep(writeCycle)
		offset += count
	}
	return nil
}

// ReadText reads the stored text from the chip. A missing magic or an
// implausible length reads as an empty text.
func (e *At24c128Eeprom) ReadText() (string, error) {
	header, err := e.read(0, headerSize)
	if err != nil {
		return "", err
	}
	length := recordLength(header)
	if length <= 0 {
		return "", nil
	}
	text, err := e.read(headerSize, length)
	if err != nil {
		return "", err
	}
	return decodeUTF8Replace(text), nil
}

// WriteText stores the UTF-8 [text] (header + payload) on the chip.
func (e *At24c128Eeprom) WriteText(text []byte) error {
	return e.write(0, encodeRecord(text))
}

func (e *At24c128Eeprom) Close() { e.f.Close() }

// -- Emulation ---------------------------------------------------------------

// EmulatedEeprom keeps the text in memory instead of driving hardware.
type EmulatedEeprom struct {
	text string
}

func NewEmulatedEeprom() *EmulatedEeprom {
	return &EmulatedEeprom{text: "Hello from the emulated EEPROM"}
}

// ReadText returns the fake stored text.
func (e *EmulatedEeprom) ReadText() (string, error) { return e.text, nil }

// WriteText stores the UTF-8 [text] in memory.
func (e *EmulatedEeprom) WriteText(text []byte) error {
	e.text = decodeUTF8Replace(text)
	fmt.Printf("[emulation] stored %d bytes\n", len(text))
	return nil
}

func (e *EmulatedEeprom) Close() {}
