// Tests for the platform-neutral driver logic: the gesture-code-to-name
// mapping (matching the Python node's GESTURES dict / the app's Gesture
// enum) and grove.py's two-phase flag decoding, with reads and sleeps
// injected.
package main

import (
	"testing"
	"time"
)

func TestGestureCodeToNameMapping(t *testing.T) {
	expected := map[int]string{
		1: "forward",
		2: "backward",
		3: "right",
		4: "left",
		5: "up",
		6: "down",
		7: "clockwise",
		8: "antiClockwise",
		9: "wave",
	}
	if len(gestureNames) != len(expected) {
		t.Fatalf("gestureNames has %d entries, want %d", len(gestureNames), len(expected))
	}
	for code, want := range expected {
		if got := gestureNames[code]; got != want {
			t.Errorf("gestureNames[%d] = %q, want %q", code, got, want)
		}
	}
	// 0 means "no gesture" and must not map to a name.
	if name, ok := gestureNames[0]; ok {
		t.Errorf("gestureNames[0] = %q, want no entry", name)
	}
}

// read is one scripted register read for the fake bus.
type read struct {
	reg uint8
	val uint8
}

// fakeReads feeds decodeGesture a scripted sequence of register reads.
func fakeReads(t *testing.T, reads []read) func(uint8) (uint8, error) {
	i := 0
	return func(reg uint8) (uint8, error) {
		if i >= len(reads) {
			t.Fatalf("unexpected read of register 0x%02x", reg)
		}
		r := reads[i]
		i++
		if reg != r.reg {
			t.Fatalf("read register 0x%02x, want 0x%02x", reg, r.reg)
		}
		return r.val, nil
	}
}

func TestDecodeGesture(t *testing.T) {
	cases := []struct {
		name  string
		reads []read
		code  int
	}{
		{"right", []read{{0x43, 0x01}, {0x43, 0x00}}, 3},
		{"left", []read{{0x43, 0x02}, {0x43, 0x00}}, 4},
		{"up", []read{{0x43, 0x04}, {0x43, 0x00}}, 5},
		{"down", []read{{0x43, 0x08}, {0x43, 0x00}}, 6},
		{"forward", []read{{0x43, 0x10}}, 1},
		{"backward", []read{{0x43, 0x20}}, 2},
		{"forward after right", []read{{0x43, 0x01}, {0x43, 0x10}}, 1},
		{"backward after up", []read{{0x43, 0x04}, {0x43, 0x20}}, 2},
		{"clockwise", []read{{0x43, 0x40}}, 7},
		{"anticlockwise", []read{{0x43, 0x80}}, 8},
		{"wave", []read{{0x43, 0x00}, {0x44, 0x01}}, 9},
		{"nothing", []read{{0x43, 0x00}, {0x44, 0x00}}, 0},
	}
	for _, c := range cases {
		got, err := decodeGesture(fakeReads(t, c.reads), func(time.Duration) {})
		if err != nil {
			t.Fatalf("%s: %v", c.name, err)
		}
		if got != c.code {
			t.Errorf("%s: code %d, want %d", c.name, got, c.code)
		}
	}
}

// A combined gesture waits GES_ENTRY_TIME before the re-read and
// GES_QUIT_TIME afterwards, exactly like grove.py.
func TestDecodeGestureTiming(t *testing.T) {
	var slept []time.Duration
	readFn := fakeReads(t, []read{{0x43, 0x01}, {0x43, 0x10}})
	code, err := decodeGesture(readFn, func(d time.Duration) { slept = append(slept, d) })
	if err != nil || code != 1 {
		t.Fatalf("code %d, err %v; want 1, nil", code, err)
	}
	want := []time.Duration{gesEntryTime, gesQuitTime}
	if len(slept) != len(want) || slept[0] != want[0] || slept[1] != want[1] {
		t.Errorf("sleeps %v, want %v", slept, want)
	}
}
