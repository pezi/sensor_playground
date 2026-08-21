package main

import "testing"

// The segment patterns the panel receives, gfedcba bit order. Golden
// values from the Python node's SEGMENT_DIGITS table; the colon is bit 7
// of the second digit and only ever set while the blink is on.
func TestSegmentsForTime(t *testing.T) {
	cases := []struct {
		name    string
		set     bool
		hour    int
		minute  int
		colonOn bool
		want    [4]byte
	}{
		{"no time yet", false, 0, 0, false, [4]byte{0x40, 0x40, 0x40, 0x40}},
		// A lit colon must not leak into the placeholder frame.
		{"no time yet, colon on", false, 0, 0, true, [4]byte{0x40, 0x40, 0x40, 0x40}},
		{"12:34 colon off", true, 12, 34, false, [4]byte{0x06, 0x5B, 0x4F, 0x66}},
		{"12:34 colon on", true, 12, 34, true, [4]byte{0x06, 0x5B | 0x80, 0x4F, 0x66}},
		{"midnight", true, 0, 0, true, [4]byte{0x3F, 0x3F | 0x80, 0x3F, 0x3F}},
		{"23:59", true, 23, 59, false, [4]byte{0x5B, 0x4F, 0x6D, 0x6F}},
		{"09:07", true, 9, 7, false, [4]byte{0x3F, 0x6F, 0x3F, 0x07}},
	}
	for _, c := range cases {
		got := segmentsForTime(c.set, c.hour, c.minute, c.colonOn)
		if got != c.want {
			t.Errorf("%s: segmentsForTime(%v, %d, %d, %v) = %#v, want %#v",
				c.name, c.set, c.hour, c.minute, c.colonOn, got, c.want)
		}
	}
}

// Every digit pattern lights only the seven segments a-g (bit 7 is the
// colon and belongs to no digit), and no two digits share a pattern.
func TestSegmentDigitsAreDistinctSevenSegment(t *testing.T) {
	seen := map[byte]int{}
	for digit, pattern := range segmentDigits {
		if pattern&0x80 != 0 {
			t.Errorf("digit %d: pattern %#02x sets the colon bit", digit, pattern)
		}
		if other, dup := seen[pattern]; dup {
			t.Errorf("digit %d shares pattern %#02x with digit %d", digit, pattern, other)
		}
		seen[pattern] = digit
	}
	if segmentDash&0x80 != 0 {
		t.Errorf("dash pattern %#02x sets the colon bit", byte(segmentDash))
	}
}

// "HH:MM" only, both fields two digits, 24-hour range.
func TestParseTimeText(t *testing.T) {
	cases := []struct {
		text   string
		hour   int
		minute int
		ok     bool
	}{
		{"00:00", 0, 0, true},
		{"12:34", 12, 34, true},
		{"23:59", 23, 59, true},
		{"24:00", 0, 0, false},
		{"23:60", 0, 0, false},
		{"9:05", 0, 0, false},
		{"09:5", 0, 0, false},
		{"0905", 0, 0, false},
		{"", 0, 0, false},
		{"aa:bb", 0, 0, false},
		{"12:34:56", 0, 0, false},
	}
	for _, c := range cases {
		hour, minute, ok := parseTimeText(c.text)
		if ok != c.ok || (ok && (hour != c.hour || minute != c.minute)) {
			t.Errorf("parseTimeText(%q) = %d, %d, %v, want %d, %d, %v",
				c.text, hour, minute, ok, c.hour, c.minute, c.ok)
		}
	}
}

// The brightness command byte the chip receives: display-on plus the
// three brightness bits, with out-of-range values clamped like the
// Python node's max(0, min(7, ...)).
func TestClampBrightness(t *testing.T) {
	cases := []struct{ in, want int }{{-5, 0}, {0, 0}, {3, 3}, {7, 7}, {8, 7}, {255, 7}}
	for _, c := range cases {
		if got := clampBrightness(c.in); got != c.want {
			t.Errorf("clampBrightness(%d) = %d, want %d", c.in, got, c.want)
		}
	}
}
