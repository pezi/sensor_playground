//go:build linux

// Minimal GPIO character-device (v2 uAPI) wrapper — one output line for
// the LED, one optional input line for the button, no dependencies.
// /sys/class/gpio is gone in Debian 13; this is its replacement.
package common

import (
	"time"

	"encoding/binary"
	"fmt"
	"os"
	"unsafe"

	"golang.org/x/sys/unix"
)

const (
	gpioV2GetLineIoctl   = 0xC250B407 // _IOWR(0xB4, 0x07, gpio_v2_line_request)
	gpioV2SetValuesIoctl = 0xC010B40F // _IOWR(0xB4, 0x0F, gpio_v2_line_values)
	gpioV2GetValuesIoctl = 0xC010B40E // _IOWR(0xB4, 0x0E, gpio_v2_line_values)
	lineRequestSize      = 592
	consumerOffset       = 256
	configFlagsOffset    = 288
	numLinesOffset       = 560
	fdOffset             = 588

	flagActiveLow    = 1 << 1
	flagInput        = 1 << 2
	flagOutput       = 1 << 3
	flagBiasPullUp   = 1 << 8
	flagBiasPullDown = 1 << 9
)

// GpioLine is one requested GPIO line (logical polarity — the kernel
// applies the active-low translation).
type GpioLine struct {
	f *os.File
}

func requestLine(chipPath string, offset uint32, flags uint64, consumer string) (*GpioLine, error) {
	chip, err := os.OpenFile(chipPath, os.O_RDWR, 0)
	if err != nil {
		return nil, err
	}
	defer chip.Close()

	var req [lineRequestSize]byte
	binary.LittleEndian.PutUint32(req[0:], offset) // offsets[0]
	copy(req[consumerOffset:consumerOffset+31], consumer)
	binary.LittleEndian.PutUint64(req[configFlagsOffset:], flags)
	binary.LittleEndian.PutUint32(req[numLinesOffset:], 1)

	if _, _, errno := unix.Syscall(unix.SYS_IOCTL, chip.Fd(), gpioV2GetLineIoctl,
		uintptr(unsafe.Pointer(&req[0]))); errno != 0 {
		return nil, fmt.Errorf("requesting GPIO line %d on %s: %v", offset, chipPath, errno)
	}
	fd := int32(binary.LittleEndian.Uint32(req[fdOffset:]))
	return &GpioLine{f: os.NewFile(uintptr(fd), fmt.Sprintf("gpio-line-%d", offset))}, nil
}

// OpenOutputLine requests a line as output. With activeLow the LED lights
// when the pin is driven LOW; Set(true) always means "light the LED".
func OpenOutputLine(chipPath string, offset int, activeLow bool) (*GpioLine, error) {
	flags := uint64(flagOutput)
	if activeLow {
		flags |= flagActiveLow
	}
	return requestLine(chipPath, uint32(offset), flags, "sensor-playground-led")
}

// OpenInputLine requests a line as input. With activeLow the line idles
// HIGH via the internal pull-up and Get() returns true when pulled low
// (pressed); otherwise a pull-down and true on HIGH — matching gpiozero's
// pull_up semantics in the Python node.
func OpenInputLine(chipPath string, offset int, activeLow bool) (*GpioLine, error) {
	flags := uint64(flagInput)
	if activeLow {
		flags |= flagActiveLow | flagBiasPullUp
	} else {
		flags |= flagBiasPullDown
	}
	return requestLine(chipPath, uint32(offset), flags, "sensor-playground-button")
}

func (l *GpioLine) ioctlValues(req uintptr, values *[16]byte) error {
	if _, _, errno := unix.Syscall(unix.SYS_IOCTL, l.f.Fd(), req,
		uintptr(unsafe.Pointer(&values[0]))); errno != 0 {
		return errno
	}
	return nil
}

// Set drives the line to the logical value.
func (l *GpioLine) Set(on bool) error {
	var values [16]byte // gpio_v2_line_values{bits, mask}
	if on {
		values[0] = 1
	}
	values[8] = 1 // mask: line 0
	return l.ioctlValues(gpioV2SetValuesIoctl, &values)
}

// Get reads the logical value of the line.
func (l *GpioLine) Get() (bool, error) {
	var values [16]byte
	values[8] = 1 // mask: line 0
	if err := l.ioctlValues(gpioV2GetValuesIoctl, &values); err != nil {
		return false, err
	}
	return values[0]&1 == 1, nil
}

func (l *GpioLine) Close() error { return l.f.Close() }

// -- Edge events --------------------------------------------------------------

const (
	flagEdgeRising  = 1 << 10
	flagEdgeFalling = 1 << 11

	gpioV2LineEventSize = 48 // struct gpio_v2_line_event
)

// GpioEvent is one edge event with the kernel's timestamp — microsecond
// pulse timing that a userspace polling loop could never achieve.
type GpioEvent struct {
	Rising      bool
	TimestampNs uint64
}

// OpenEventLine requests an input line reporting both edges with kernel
// timestamps. pullUp/pullDown select the internal bias (both false: as-is).
func OpenEventLine(chipPath string, offset int, activeLow, pullUp, pullDown bool) (*GpioLine, error) {
	flags := uint64(flagInput | flagEdgeRising | flagEdgeFalling)
	if activeLow {
		flags |= flagActiveLow
	}
	if pullUp {
		flags |= flagBiasPullUp
	}
	if pullDown {
		flags |= flagBiasPullDown
	}
	return requestLine(chipPath, uint32(offset), flags, "sensor-playground-events")
}

// ReadEvent blocks until the next edge event or the timeout (nil event on
// timeout). Events are queued by the kernel, so bursts are not lost as
// long as they are drained reasonably quickly.
func (l *GpioLine) ReadEvent(timeout time.Duration) (*GpioEvent, error) {
	fds := []unix.PollFd{{Fd: int32(l.f.Fd()), Events: unix.POLLIN}}
	n, err := unix.Poll(fds, int(timeout.Milliseconds()))
	if err != nil || n == 0 {
		return nil, err
	}
	buf := make([]byte, gpioV2LineEventSize)
	if _, err := l.f.Read(buf); err != nil {
		return nil, err
	}
	return &GpioEvent{
		Rising:      binary.LittleEndian.Uint32(buf[8:]) == 1, // id: 1 rising, 2 falling
		TimestampNs: binary.LittleEndian.Uint64(buf[0:]),
	}, nil
}
