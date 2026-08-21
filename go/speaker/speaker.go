// Outputs for the speaker node: a real GPIO line driven with a square wave
// and an emulated one that just prints.
//
// The Grove Speaker is a small amplified loudspeaker on a plain digital
// pin, so a tone is a 50%-duty square wave of the desired pitch. The Python
// node gets that from lgpio's tx_pwm, which is itself a software PWM thread
// — this port does the same thing directly: one goroutine toggles the line
// and sleeps half a period, and it is restarted (not signalled) on every
// pitch change so the timing loop stays a tight two-liner.
//
// Timing comes from the scheduler, so the pitch is only as steady as the
// system's sleep granularity; expect audible jitter under load. That is
// good enough for a beeper — it is not a music synthesiser.
package main

import (
	"fmt"
	"sync"
	"time"

	"sensorplayground/common"
)

// Output drives the speaker at a frequency, or silences it at 0 Hz.
type Output interface {
	Play(frequency int)
	Close()
}

// EmulatedOutput prints the sounding state instead of driving hardware.
type EmulatedOutput struct{}

func (o *EmulatedOutput) Play(frequency int) {
	state := "silent"
	if frequency > 0 {
		state = fmt.Sprintf("%d Hz", frequency)
	}
	fmt.Printf("[emulation] speaker %s\n", state)
}

func (o *EmulatedOutput) Close() {}

// PwmOutput drives the speaker with a 50%-duty square wave on a GPIO
// character-device line (/dev/gpiochipN — /sys/class/gpio is gone in
// Debian 13).
type PwmOutput struct {
	line *common.GpioLine

	mu      sync.Mutex
	stop    chan struct{}  // closed to stop the current tone
	done    sync.WaitGroup // waits for the toggling goroutine to finish
	stopped bool
}

// NewPwmOutput claims the speaker pin as an output, silent.
func NewPwmOutput(gpioChip, pin int) (*PwmOutput, error) {
	line, err := common.OpenOutputLine(fmt.Sprintf("/dev/gpiochip%d", gpioChip), pin, false)
	if err != nil {
		return nil, err
	}
	return &PwmOutput{line: line}, nil
}

// Play sounds [frequency] Hz, or silences the pin when it is 0.
func (o *PwmOutput) Play(frequency int) {
	o.mu.Lock()
	defer o.mu.Unlock()
	o.silence()
	if o.stopped || frequency <= 0 {
		return
	}
	// Half a period per level, so one full cycle is 1/frequency seconds.
	halfPeriod := time.Duration(float64(time.Second) / float64(frequency) / 2)
	stop := make(chan struct{})
	o.stop = stop
	o.done.Add(1)
	go func() {
		defer o.done.Done()
		level := false
		for {
			select {
			case <-stop:
				o.line.Set(false)
				return
			default:
			}
			level = !level
			o.line.Set(level)
			time.Sleep(halfPeriod)
		}
	}()
}

// silence stops the toggling goroutine and leaves the pin low. The caller
// holds the lock.
func (o *PwmOutput) silence() {
	if o.stop == nil {
		return
	}
	close(o.stop)
	o.stop = nil
	o.done.Wait()
	o.line.Set(false)
}

func (o *PwmOutput) Close() {
	o.mu.Lock()
	defer o.mu.Unlock()
	o.silence()
	o.stopped = true
	o.line.Close()
}
