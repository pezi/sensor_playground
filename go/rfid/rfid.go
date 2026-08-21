// RDM630 frame parsing for the Grove 125KHz RFID Reader — portable, so it
// can be unit-tested on any machine.
//
// Frame format (reader TX, jumper on UART mode — not Wiegand):
//
//	STX 0x02 | 10 ASCII-hex data chars | 2 ASCII-hex checksum chars | ETX 0x03
//
// The checksum byte is the XOR of the five data bytes.
package main

import (
	"strconv"
	"strings"
)

const (
	stx           = 0x02
	etx           = 0x03
	frameHexChars = 12 // 10 data chars + 2 checksum chars
)

// FrameParser is the byte-wise state machine for RDM630-style frames.
type FrameParser struct {
	frame   []byte
	inFrame bool // false = waiting for STX, else collecting hex chars
}

// Feed advances the state machine by one byte; it returns a validated
// 10-char hex tag whenever a byte completes a frame.
func (p *FrameParser) Feed(b byte) (string, bool) {
	if b == stx {
		p.frame = p.frame[:0] // resync, also on a second STX
		p.inFrame = true
		return "", false
	}
	if !p.inFrame {
		return "", false // noise outside a frame
	}
	if b == etx {
		p.inFrame = false
		if len(p.frame) == frameHexChars && checksumOK(p.frame) {
			return strings.ToUpper(string(p.frame[:10])), true
		}
		return "", false
	}
	if isHexChar(b) {
		p.frame = append(p.frame, b)
		if len(p.frame) > frameHexChars {
			p.inFrame = false // overflow: wait for the next STX
		}
	} else {
		p.inFrame = false // non-hex noise mid-frame
	}
	return "", false
}

func isHexChar(b byte) bool {
	return (b >= '0' && b <= '9') || (b >= 'A' && b <= 'F') || (b >= 'a' && b <= 'f')
}

// checksumOK: the XOR of the five data bytes must equal the checksum byte.
func checksumOK(frame []byte) bool {
	var values [frameHexChars / 2]byte
	for i := 0; i < frameHexChars; i += 2 {
		v, err := strconv.ParseUint(string(frame[i:i+2]), 16, 8)
		if err != nil {
			return false
		}
		values[i/2] = byte(v)
	}
	var checksum byte
	for _, v := range values[:5] {
		checksum ^= v
	}
	return checksum == values[5]
}
