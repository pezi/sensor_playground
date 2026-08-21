//go:build !linux

// The real CozIR needs a Linux UART; on other platforms only emulation
// mode is available.
package main

import "errors"

type SerialPort struct{}

func OpenSerial(path string) (*SerialPort, error) {
	return nil, errors.New("the CozIR serial port requires Linux (use \"emulation\": true elsewhere)")
}

func (p *SerialPort) Write(data []byte) error   { return nil }
func (p *SerialPort) FlushInput()               {}
func (p *SerialPort) ReadLine(limit int) string { return "" }
func (p *SerialPort) Close() error              { return nil }
