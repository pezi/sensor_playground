//go:build !linux

// The real Air530 needs a Linux UART; on other platforms only emulation
// mode is available.
package main

import "errors"

type SerialPort struct{}

func OpenSerial(path string) (*SerialPort, error) {
	return nil, errors.New("the Air530 serial port requires Linux (use \"emulation\": true elsewhere)")
}

func (p *SerialPort) FlushInput()                {}
func (p *SerialPort) ReadBlock(limit int) string { return "" }
func (p *SerialPort) Close() error               { return nil }
