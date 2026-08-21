// The command handling and playback state must match the Python node's:
// out-of-range tones are ignored, the melody steps through its notes, and
// every accepted command marks the state for publication.
package main

import (
	"testing"
	"time"
)

// recordingOutput captures what the controller asked the speaker to sound.
type recordingOutput struct{ played []int }

func (o *recordingOutput) Play(frequency int) { o.played = append(o.played, frequency) }
func (o *recordingOutput) Close()             {}

func newTestSpeaker() (*SpeakerController, *recordingOutput) {
	output := &recordingOutput{}
	speaker := NewSpeakerController(output)
	speaker.TakePending() // drop the initial state
	return speaker, output
}

func TestPlayToneAcceptsAudibleTones(t *testing.T) {
	speaker, output := newTestSpeaker()
	handleJSONCommand(speaker, []byte(`{"tone": {"freq": 440, "ms": 400}}`))
	if got := speaker.State()["freq"]; got != 440 {
		t.Fatalf("freq = %v, want 440", got)
	}
	if !speaker.TakePending() {
		t.Fatal("an accepted tone must mark the state pending")
	}
	if len(output.played) == 0 || output.played[len(output.played)-1] != 440 {
		t.Fatalf("output.played = %v, want it to end at 440", output.played)
	}
}

func TestPlayToneIgnoresOutOfRangeAndMalformed(t *testing.T) {
	for _, command := range []string{
		`{"tone": {"freq": 19, "ms": 400}}`,    // below the audible range
		`{"tone": {"freq": 20001, "ms": 400}}`, // above it
		`{"tone": {"freq": 440, "ms": 0}}`,     // no duration
		`{"tone": {"freq": 440.5, "ms": 400}}`, // not an integer
		`{"tone": {"freq": 440}}`,              // no duration at all
		`{"nonsense": true}`,
		`not json`,
	} {
		speaker, _ := newTestSpeaker()
		handleJSONCommand(speaker, []byte(command))
		if got := speaker.State()["freq"]; got != 0 {
			t.Fatalf("%s: freq = %v, want 0 (ignored)", command, got)
		}
		if speaker.TakePending() {
			t.Fatalf("%s: an ignored command must not mark the state pending", command)
		}
	}
}

func TestMelodyStepsThroughItsNotes(t *testing.T) {
	speaker, _ := newTestSpeaker()
	handleJSONCommand(speaker, []byte(`{"melody": true}`))
	if got := speaker.State()["freq"]; got != melody[0].frequency {
		t.Fatalf("freq = %v, want the first note %d", got, melody[0].frequency)
	}
	// Force each note to expire and check the controller walks the tune and
	// falls silent at the end.
	for _, want := range append(frequenciesAfterFirst(), 0) {
		speaker.deadline = speaker.deadline.Add(-time.Hour)
		speaker.Tick()
		if got := speaker.State()["freq"]; got != want {
			t.Fatalf("after a step freq = %v, want %d", got, want)
		}
	}
	// Silence is the end: another tick must not restart anything.
	speaker.Tick()
	if got := speaker.State()["freq"]; got != 0 {
		t.Fatalf("freq = %v after the melody ended, want 0", got)
	}
}

func frequenciesAfterFirst() []int {
	frequencies := make([]int, 0, len(melody)-1)
	for _, n := range melody[1:] {
		frequencies = append(frequencies, n.frequency)
	}
	return frequencies
}

func TestStopSilencesImmediately(t *testing.T) {
	speaker, _ := newTestSpeaker()
	handleJSONCommand(speaker, []byte(`{"melody": true}`))
	handleJSONCommand(speaker, []byte(`{"stop": true}`))
	if got := speaker.State()["freq"]; got != 0 {
		t.Fatalf("freq = %v after stop, want 0", got)
	}
	if !speaker.TakePending() {
		t.Fatal("stop must mark the state pending")
	}
	// A stopped melody must not resume on the next tick.
	speaker.Tick()
	if got := speaker.State()["freq"]; got != 0 {
		t.Fatalf("freq = %v after a tick, want 0", got)
	}
}
