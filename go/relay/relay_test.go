package main

import (
	"errors"
	"reflect"
	"testing"
)

type recordingBank struct {
	count  int
	writes [][]bool
	fail   bool
}

func (b *recordingBank) Count() int { return b.count }
func (b *recordingBank) Write(states []bool) error {
	if b.fail {
		return errors.New("write failed")
	}
	b.writes = append(b.writes, append([]bool(nil), states...))
	return nil
}
func (b *recordingBank) Close() error { return nil }

func newTestRelay(t *testing.T, channels int) (*RelayController, *recordingBank) {
	t.Helper()
	bank := &recordingBank{count: channels}
	relay, err := NewRelayController(bank)
	if err != nil {
		t.Fatal(err)
	}
	relay.TakePending()
	return relay, bank
}

func relayStates(t *testing.T, relay *RelayController) []bool {
	t.Helper()
	states, ok := relay.Payload()["relay"].([]bool)
	if !ok {
		t.Fatal("relay payload has no []bool state")
	}
	return states
}

func TestRelayCommandsAndPayload(t *testing.T) {
	relay, bank := newTestRelay(t, 2)
	handleJSONCommand(relay, []byte(`{"ch":0,"on":true}`))
	handleJSONCommand(relay, []byte(`{"ch":1,"toggle":true}`))
	if got, want := relayStates(t, relay), []bool{true, true}; !reflect.DeepEqual(got, want) {
		t.Fatalf("states = %v, want %v", got, want)
	}
	handleJSONCommand(relay, []byte(`{"all":false}`))
	if got, want := relayStates(t, relay), []bool{false, false}; !reflect.DeepEqual(got, want) {
		t.Fatalf("states = %v, want %v", got, want)
	}
	if len(bank.writes) != 4 { // startup + set + toggle + all
		t.Fatalf("writes = %d, want 4", len(bank.writes))
	}
	payload, pending := relay.TakePending()
	if !pending || payload["channels"] != 2 {
		t.Fatalf("pending payload = %#v, %v", payload, pending)
	}
}

func TestMalformedAndOutOfRangeCommandsAreIgnored(t *testing.T) {
	for _, command := range []string{
		`not json`, `{"all":true} trailing`, `{"ch":2,"on":true}`, `{"ch":-1,"on":true}`,
		`{"ch":0.5,"on":true}`, `{"ch":true,"on":true}`,
		`{"ch":0,"on":1}`, `{"toggle":true}`,
	} {
		relay, bank := newTestRelay(t, 2)
		handleJSONCommand(relay, []byte(command))
		if len(bank.writes) != 1 {
			t.Fatalf("%s caused a hardware write", command)
		}
		if _, pending := relay.TakePending(); pending {
			t.Fatalf("%s marked state pending", command)
		}
	}
}

func TestFailedWriteDoesNotCommitState(t *testing.T) {
	relay, bank := newTestRelay(t, 1)
	bank.fail = true
	if err := relay.Set(0, true); err == nil {
		t.Fatal("failed bank write returned nil")
	}
	if relayStates(t, relay)[0] {
		t.Fatal("failed hardware write was committed")
	}
	if _, pending := relay.TakePending(); pending {
		t.Fatal("failed hardware write was marked pending")
	}
}

func TestI2CRelayMask(t *testing.T) {
	if got, want := relayMask([]bool{true, false, true, true}), uint8(0x0d); got != want {
		t.Fatalf("mask = %#02x, want %#02x", got, want)
	}
	if got, err := parseI2CAddress("0x11"); err != nil || got != 0x11 {
		t.Fatalf("parseI2CAddress(0x11) = %#02x, %v", got, err)
	}
	if _, err := parseI2CAddress("0x80"); err == nil {
		t.Fatal("parseI2CAddress accepted a non-7-bit address")
	}
}
