//go:build !linux

// The real RFID reader needs a Linux UART; on other platforms only
// emulation mode is available.
package main

import "errors"

type SerialPort struct{}

func OpenSerial(path string) (*SerialPort, error) {
	return nil, errors.New("the RFID reader serial port requires Linux (use \"emulation\": true elsewhere)")
}

func (p *SerialPort) Read(buf []byte) (int, error) { return 0, nil }
func (p *SerialPort) Close() error                 { return nil }
