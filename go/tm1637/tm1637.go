// TM1637 display driver: segment encoding plus the bit-banged two-wire
// bus. The TM1637 speaks a proprietary protocol (start/stop conditions
// like I2C, but LSB-first and without addresses) and has no minimum
// clock speed, so it is bit-banged on two GPIO character-device lines —
// one ioctl per transition paces the bus well below the chip's limit.
package main

import (
	"sensorplayground/common"
)

// Segment patterns for the digits 0-9, gfedcba bit order.
var segmentDigits = [10]byte{0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F}

const (
	// A lone middle segment (g), the "no time yet" placeholder digit.
	segmentDash = 0x40
	// The Grove module wires the colon to bit 7 of the second digit.
	segmentColon = 0x80
)

// segmentsForTime returns the four digit patterns for the panel — dashes
// when set is false (no time yet), otherwise HH:MM with the colon on the
// second digit when colonOn.
func segmentsForTime(set bool, hour, minute int, colonOn bool) [4]byte {
	if !set {
		return [4]byte{segmentDash, segmentDash, segmentDash, segmentDash}
	}
	segments := [4]byte{
		segmentDigits[hour/10],
		segmentDigits[hour%10],
		segmentDigits[minute/10],
		segmentDigits[minute%10],
	}
	if colonOn {
		segments[1] |= segmentColon
	}
	return segments
}

// GpioDisplay bit-bangs a TM1637 4-digit display on two GPIO lines.
type GpioDisplay struct {
	clk *common.GpioLine
	dio *common.GpioLine
}

func NewGpioDisplay(chipPath string, clkPin, dioPin int) (*GpioDisplay, error) {
	clk, err := common.OpenOutputLine(chipPath, clkPin, false)
	if err != nil {
		return nil, err
	}
	dio, err := common.OpenOutputLine(chipPath, dioPin, false)
	if err != nil {
		clk.Close()
		return nil, err
	}
	return &GpioDisplay{clk: clk, dio: dio}, nil
}

func (d *GpioDisplay) start() {
	// DIO falls while CLK is high.
	d.clk.Set(true)
	d.dio.Set(true)
	d.dio.Set(false)
	d.clk.Set(false)
}

func (d *GpioDisplay) stop() {
	// DIO rises while CLK is high.
	d.clk.Set(false)
	d.dio.Set(false)
	d.clk.Set(true)
	d.dio.Set(true)
}

func (d *GpioDisplay) writeByte(value byte) {
	for bit := 0; bit < 8; bit++ {
		d.clk.Set(false)
		d.dio.Set((value>>bit)&1 == 1)
		d.clk.Set(true)
	}
	// ACK slot: the chip acknowledges by pulling DIO low itself. Like the
	// Python node, drive DIO low through the ninth clock pulse instead of
	// re-requesting the line as input — the same level the chip is
	// asserting, so nothing conflicts and there is nothing useful to do
	// on a NAK anyway.
	d.clk.Set(false)
	d.dio.Set(false)
	d.clk.Set(true)
	d.clk.Set(false)
}

// Render writes the time (or dashes when set is false) to the panel.
func (d *GpioDisplay) Render(set bool, hour, minute int, colonOn bool, brightness int) {
	segments := segmentsForTime(set, hour, minute, colonOn)

	d.start()
	d.writeByte(0x40) // data command: write, auto-increment address
	d.stop()

	d.start()
	d.writeByte(0xC0) // address command: start at digit 0
	for _, segment := range segments {
		d.writeByte(segment)
	}
	d.stop()

	d.start()
	d.writeByte(0x88 | byte(brightness&0x07)) // display on
	d.stop()
}

func (d *GpioDisplay) Close() {
	// Blank the panel rather than leaving a stale time burning.
	d.Render(false, 0, 0, false, 0)
	d.clk.Close()
	d.dio.Close()
}
