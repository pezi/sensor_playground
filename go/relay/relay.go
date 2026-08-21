package main

import (
	"fmt"
	"sync"

	"sensorplayground/common"
)

const i2cCommandChannelControl = 0x10

// RelayBank drives a complete relay board. Writes always contain the state
// of every channel, which matches both the GPIO modules and the I2C module's
// bitmask command.
type RelayBank interface {
	Count() int
	Write(states []bool) error
	Close() error
}

// RelayController owns the state reported to clients. A state is committed
// only after the board accepted it, so a failed hardware write cannot be
// advertised as successful.
type RelayController struct {
	mu      sync.Mutex
	bank    RelayBank
	states  []bool
	pending bool
}

func NewRelayController(bank RelayBank) (*RelayController, error) {
	if bank.Count() < 1 || bank.Count() > 8 {
		return nil, fmt.Errorf("relay board must have 1..8 channels (got %d)", bank.Count())
	}
	r := &RelayController{
		bank:    bank,
		states:  make([]bool, bank.Count()),
		pending: true,
	}
	if err := bank.Write(r.states); err != nil {
		return nil, fmt.Errorf("switching all relays off at startup: %w", err)
	}
	return r, nil
}

func (r *RelayController) Count() int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return len(r.states)
}

func (r *RelayController) applyLocked(next []bool) error {
	if err := r.bank.Write(next); err != nil {
		return err
	}
	r.states = next
	r.pending = true
	return nil
}

func (r *RelayController) Set(channel int, on bool) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if channel < 0 || channel >= len(r.states) {
		return fmt.Errorf("channel %d is outside board width %d", channel, len(r.states))
	}
	next := append([]bool(nil), r.states...)
	next[channel] = on
	return r.applyLocked(next)
}

func (r *RelayController) Toggle(channel int) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	if channel < 0 || channel >= len(r.states) {
		return fmt.Errorf("channel %d is outside board width %d", channel, len(r.states))
	}
	next := append([]bool(nil), r.states...)
	next[channel] = !next[channel]
	return r.applyLocked(next)
}

func (r *RelayController) SetAll(on bool) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	next := make([]bool, len(r.states))
	if on {
		for channel := range next {
			next[channel] = true
		}
	}
	return r.applyLocked(next)
}

func payloadFor(states []bool) map[string]any {
	return map[string]any{
		"channels": len(states),
		"relay":    append([]bool(nil), states...),
	}
}

func (r *RelayController) Payload() map[string]any {
	r.mu.Lock()
	defer r.mu.Unlock()
	return payloadFor(r.states)
}

// TakePending returns one atomic state snapshot after each accepted command.
func (r *RelayController) TakePending() (map[string]any, bool) {
	r.mu.Lock()
	defer r.mu.Unlock()
	if !r.pending {
		return nil, false
	}
	r.pending = false
	return payloadFor(r.states), true
}

func (r *RelayController) Close() error {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.bank.Close()
}

// -- GPIO relay bank ---------------------------------------------------------

type gpioRelayBank struct {
	lines []*common.GpioLine
}

func newGPIOBank(chip string, pins []int, activeLow bool) (*gpioRelayBank, error) {
	if len(pins) < 1 || len(pins) > 8 {
		return nil, fmt.Errorf("relay_pins must list 1..8 GPIO lines")
	}
	bank := &gpioRelayBank{}
	for _, pin := range pins {
		line, err := common.OpenOutputLine(chip, pin, activeLow)
		if err != nil {
			bank.Close()
			return nil, fmt.Errorf("opening relay GPIO %d: %w", pin, err)
		}
		bank.lines = append(bank.lines, line)
	}
	return bank, nil
}

func (b *gpioRelayBank) Count() int { return len(b.lines) }

func (b *gpioRelayBank) Write(states []bool) error {
	if len(states) != len(b.lines) {
		return fmt.Errorf("got %d relay states for %d GPIO lines", len(states), len(b.lines))
	}
	for channel, line := range b.lines {
		if err := line.Set(states[channel]); err != nil {
			return fmt.Errorf("writing relay channel %d: %w", channel, err)
		}
	}
	return nil
}

func (b *gpioRelayBank) Close() error {
	var first error
	for _, line := range b.lines {
		if err := line.Close(); err != nil && first == nil {
			first = err
		}
	}
	return first
}

// -- Grove 4-channel I2C relay bank -----------------------------------------

type i2cRelayBank struct {
	device *common.I2CDevice
	count  int
}

func newI2CBank(bus int, address uint8, channels int) (*i2cRelayBank, error) {
	if channels < 1 || channels > 8 {
		return nil, fmt.Errorf("channels must be 1..8 for the I2C relay board")
	}
	device, err := common.OpenI2C(bus, address)
	if err != nil {
		return nil, fmt.Errorf("opening /dev/i2c-%d address 0x%02x: %w", bus, address, err)
	}
	return &i2cRelayBank{device: device, count: channels}, nil
}

func (b *i2cRelayBank) Count() int { return b.count }

func relayMask(states []bool) uint8 {
	var mask uint8
	for channel, on := range states {
		if on {
			mask |= 1 << channel
		}
	}
	return mask
}

func (b *i2cRelayBank) Write(states []bool) error {
	if len(states) != b.count {
		return fmt.Errorf("got %d relay states for %d I2C channels", len(states), b.count)
	}
	return b.device.WriteReg(i2cCommandChannelControl, relayMask(states))
}

func (b *i2cRelayBank) Close() error { return b.device.Close() }

// -- Emulation ---------------------------------------------------------------

type emulatedRelayBank struct{ count int }

func (b *emulatedRelayBank) Count() int { return b.count }

func (b *emulatedRelayBank) Write(states []bool) error {
	for channel, on := range states {
		fmt.Printf("[emulation] relay %d: %s\n", channel+1, map[bool]string{true: "on", false: "off"}[on])
	}
	return nil
}

func (b *emulatedRelayBank) Close() error { return nil }
