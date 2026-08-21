package main

import "testing"

// The pulse-width -> distance conversion: sound travels 0.343 mm/µs and
// the echo pulse covers the distance twice, so 1 mm is ~5831 ns of pulse.
// Golden values from the Python node's formula.
func TestPulseToMM(t *testing.T) {
	cases := []struct {
		pulseNs uint64
		want    *int
	}{
		{5830904, intPtr(1000)},  // 1 m target
		{116618, intPtr(20)},     // exactly the 2 cm minimum
		{20408163, intPtr(3500)}, // exactly the 3.5 m maximum
		{58309, nil},             // 10 mm — below the minimum range
		{25000000, nil},          // ~4.3 m — beyond the maximum range
	}
	for _, c := range cases {
		got := pulseToMM(c.pulseNs)
		if (got == nil) != (c.want == nil) || (got != nil && *got != *c.want) {
			t.Errorf("pulseToMM(%d) = %v, want %v", c.pulseNs, fmtPtr(got), fmtPtr(c.want))
		}
	}
}

// The push policy: the first reading, an echo/no-echo flip, or a change of
// at least minDeltaMM publish immediately (the heartbeat is time-based and
// tested implicitly by the loop).
func TestShouldPublish(t *testing.T) {
	cases := []struct {
		name          string
		everPublished bool
		lastSent, mm  *int
		want          bool
	}{
		{"first reading", false, nil, intPtr(500), true},
		{"first reading no echo", false, nil, nil, true},
		{"unchanged", true, intPtr(500), intPtr(500), false},
		{"below delta", true, intPtr(500), intPtr(504), false},
		{"at delta", true, intPtr(500), intPtr(505), true},
		{"at delta downward", true, intPtr(500), intPtr(495), true},
		{"echo lost", true, intPtr(500), nil, true},
		{"echo regained", true, nil, intPtr(500), true},
		{"still no echo", true, nil, nil, false},
	}
	for _, c := range cases {
		if got := shouldPublish(c.everPublished, c.lastSent, c.mm); got != c.want {
			t.Errorf("%s: shouldPublish(%v, %v, %v) = %v, want %v",
				c.name, c.everPublished, fmtPtr(c.lastSent), fmtPtr(c.mm), got, c.want)
		}
	}
}

func intPtr(v int) *int { return &v }

func fmtPtr(v *int) any {
	if v == nil {
		return nil
	}
	return *v
}
