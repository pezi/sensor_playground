// SSD1306 128x64 OLED driver — a port of the Python node's, which is
// itself a port of dart_periphery's SSD1306: the same init sequence, the
// same horizontal-addressing window, and the same bitmap transposition.
//
// The bitmap transposition is platform-neutral (and unit tested); only the
// I2C access needs hardware.
package main

import (
	"fmt"

	"sensorplayground/common"
)

const (
	displayWidth  = 128
	displayHeight = 64
	// One bit per pixel: 1024 bytes for the whole panel.
	frameSize = displayWidth * displayHeight / 8
	// 16 bytes per row in the app's (horizontal) bitmap format.
	rowBytes = displayWidth / 8

	controlCommand = 0x00 // the byte that follows is a command
	controlData    = 0x40 // the bytes that follow are display RAM
)

// initSequence is the same one the dart_periphery SSD1306 driver sends.
var initSequence = []byte{
	0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00,
	0xA1, 0xC8, 0xDA, 0x12, 0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6,
	0xAF,
}

// toNativeFormat transposes a horizontal MSB-first bitmap into SSD1306
// page format.
//
// The app sends what https://javl.github.io/image2cpp/ calls "horizontal"
// byte orientation: 16 bytes per row, the MSB of each byte is the leftmost
// pixel. The panel wants 8 pages of 128 column bytes, each byte holding 8
// *vertical* pixels with the topmost in the LSB — so every output byte is
// assembled from one bit of eight different input rows.
func toNativeFormat(data []byte) []byte {
	buffer := make([]byte, frameSize)
	count := 0
	index := 0
	for page := 0; page < displayHeight/8; page++ {
		for columnByte := 0; columnByte < rowBytes; columnByte++ {
			pos := index + columnByte
			for bit := 7; bit >= 0; bit-- {
				mask := byte(1 << bit)
				var value byte
				for row := 0; row < 8; row++ {
					if data[pos+rowBytes*row]&mask != 0 {
						value |= 1 << row
					}
				}
				buffer[count] = value
				count++
			}
		}
		index += displayWidth
	}
	return buffer
}

// -- Displays -------------------------------------------------------------

// Display is what the command handlers drive: the real panel or the
// emulated one.
type Display interface {
	Clear() error
	ShowBitmap(data []byte) error
}

// EmulatedDisplay prints what would be drawn instead of driving hardware.
// A 1024-byte frame is meaningless on a console, so it reports the share
// of lit pixels — enough to tell a blank frame from a drawn one.
type EmulatedDisplay struct{}

func (d *EmulatedDisplay) Clear() error {
	fmt.Println("[emulation] display cleared")
	return nil
}

func (d *EmulatedDisplay) ShowBitmap(data []byte) error {
	lit := 0
	for _, b := range toNativeFormat(data) {
		for bit := 0; bit < 8; bit++ {
			if b&(1<<bit) != 0 {
				lit++
			}
		}
	}
	fmt.Printf("[emulation] display bitmap: %d of %d pixels lit\n",
		lit, displayWidth*displayHeight)
	return nil
}

// SSD1306 drives the panel over I2C.
type SSD1306 struct {
	dev *common.I2CDevice
}

// NewSSD1306 opens the panel on /dev/i2c-<bus>, initializes it and blanks
// it.
func NewSSD1306(bus int, address uint8) (*SSD1306, error) {
	dev, err := common.OpenI2C(bus, address)
	if err != nil {
		return nil, err
	}
	d := &SSD1306{dev: dev}
	if err := d.write(controlCommand, initSequence); err != nil {
		return nil, err
	}
	return d, d.Clear()
}

func (d *SSD1306) Close() error { return d.dev.Close() }

// write sends a control byte followed by its payload in one transfer.
func (d *SSD1306) write(control byte, data []byte) error {
	message := make([]byte, 0, len(data)+1)
	message = append(message, control)
	message = append(message, data...)
	return d.dev.WriteBytes(message)
}

// resetPosition points the panel's write cursor back at the top left by
// setting the full column (0-127) and page (0-7) range for horizontal
// addressing mode.
func (d *SSD1306) resetPosition() error {
	return d.write(controlCommand, []byte{0x21, 0x00, 0x7F, 0x22, 0x00, 0x07})
}

// Clear blanks the display.
func (d *SSD1306) Clear() error {
	if err := d.resetPosition(); err != nil {
		return err
	}
	return d.write(controlData, make([]byte, frameSize))
}

// ShowBitmap displays a horizontal MSB-first bitmap (1024 bytes).
func (d *SSD1306) ShowBitmap(data []byte) error {
	if err := d.resetPosition(); err != nil {
		return err
	}
	return d.write(controlData, toNativeFormat(data))
}
