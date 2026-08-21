//go:build !linux

// Real GPIO needs Linux; on other platforms only emulation mode works.
package common

import (
	"errors"
	"time"
)

type GpioLine struct{}

var errNotLinux = errors.New("GPIO requires Linux (use \"emulation\": true elsewhere)")

func OpenOutputLine(chipPath string, offset int, activeLow bool) (*GpioLine, error) {
	return nil, errNotLinux
}
func OpenInputLine(chipPath string, offset int, activeLow bool) (*GpioLine, error) {
	return nil, errNotLinux
}
func (l *GpioLine) Set(on bool) error  { return nil }
func (l *GpioLine) Get() (bool, error) { return false, nil }
func (l *GpioLine) Close() error       { return nil }

type GpioEvent struct {
	Rising      bool
	TimestampNs uint64
}

func OpenEventLine(chipPath string, offset int, activeLow, pullUp, pullDown bool) (*GpioLine, error) {
	return nil, errNotLinux
}
func (l *GpioLine) ReadEvent(timeout time.Duration) (*GpioEvent, error) { return nil, nil }
