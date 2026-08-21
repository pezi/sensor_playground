//go:build linux

// Minimal 9600-8N1 serial port via termios — just enough to read the
// Air530's continuous NMEA stream, without a serial-port dependency.
package main

import (
	"os"

	"golang.org/x/sys/unix"
)

type SerialPort struct {
	f *os.File
}

// OpenSerial opens path at 9600 baud, raw 8N1, with a 1 s read timeout
// (VTIME=10, VMIN=0 — read returns what arrived, or nothing after 1 s).
func OpenSerial(path string) (*SerialPort, error) {
	f, err := os.OpenFile(path, os.O_RDWR|unix.O_NOCTTY, 0)
	if err != nil {
		return nil, err
	}
	t, err := unix.IoctlGetTermios(int(f.Fd()), unix.TCGETS)
	if err != nil {
		f.Close()
		return nil, err
	}
	t.Iflag = 0
	t.Oflag = 0
	t.Lflag = 0
	t.Cflag = unix.CS8 | unix.CREAD | unix.CLOCAL | unix.B9600
	t.Ispeed = unix.B9600
	t.Ospeed = unix.B9600
	t.Cc[unix.VMIN] = 0
	t.Cc[unix.VTIME] = 10 // deciseconds
	if err := unix.IoctlSetTermios(int(f.Fd()), unix.TCSETS, t); err != nil {
		f.Close()
		return nil, err
	}
	return &SerialPort{f: f}, nil
}

// FlushInput discards everything already received but not yet read.
func (p *SerialPort) FlushInput() {
	unix.IoctlSetInt(int(p.f.Fd()), unix.TCFLSH, unix.TCIFLUSH)
}

// ReadBlock reads up to limit bytes of the NMEA stream — like the Python
// node's serial.read(512) — returning early after 1 s of silence.
func (p *SerialPort) ReadBlock(limit int) string {
	block := make([]byte, 0, limit)
	buf := make([]byte, 256)
	for len(block) < limit {
		n, err := p.f.Read(buf)
		if err != nil || n == 0 {
			break // timeout
		}
		if len(block)+n > limit {
			n = limit - len(block)
		}
		block = append(block, buf[:n]...)
	}
	return string(block)
}

func (p *SerialPort) Close() error { return p.f.Close() }
