//go:build linux

// Minimal 9600-8N1 serial port via termios — read side only: the Grove
// RFID reader is transmit-only, the board never sends it anything.
package main

import (
	"os"

	"golang.org/x/sys/unix"
)

type SerialPort struct {
	f *os.File
}

// OpenSerial opens path at 9600 baud, raw 8N1, with blocking reads
// (VMIN=1, VTIME=0 — a read waits until at least one byte arrived; the
// reader pushes frames asynchronously whenever a tag is presented).
func OpenSerial(path string) (*SerialPort, error) {
	f, err := os.OpenFile(path, os.O_RDONLY|unix.O_NOCTTY, 0)
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
	t.Cc[unix.VMIN] = 1
	t.Cc[unix.VTIME] = 0
	if err := unix.IoctlSetTermios(int(f.Fd()), unix.TCSETS, t); err != nil {
		f.Close()
		return nil, err
	}
	return &SerialPort{f: f}, nil
}

// Read blocks until at least one byte arrived, then returns what is there
// (up to len(buf) bytes).
func (p *SerialPort) Read(buf []byte) (int, error) { return p.f.Read(buf) }

func (p *SerialPort) Close() error { return p.f.Close() }
