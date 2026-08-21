package main

import "testing"

// Timeout register decoding/encoding ("(LSByte * 2^MSByte) + 1"); 0x0096
// and 0x01FE are the pre/final range timeouts the tuning settings write.
func TestTimeoutCoding(t *testing.T) {
	cases := []struct {
		reg   uint16
		mclks float64
	}{
		{0x0096, 151},
		{0x01FE, 509},
		{0x0180, 257},
	}
	for _, c := range cases {
		if got := vl53l0xDecodeTimeout(c.reg); got != c.mclks {
			t.Errorf("decode(0x%04X) = %v, want %v", c.reg, got, c.mclks)
		}
		if got := vl53l0xEncodeTimeout(c.mclks); got != c.reg {
			t.Errorf("encode(%v) = 0x%04X, want 0x%04X", c.mclks, got, c.reg)
		}
	}
	if got := vl53l0xEncodeTimeout(0); got != 0 {
		t.Errorf("encode(0) = 0x%04X, want 0", got)
	}
}

// MCLK <-> microsecond conversion for the tuning-default VCSEL periods
// (pre-range register 0x06 -> 14 PCLKs, final-range 0x04 -> 10 PCLKs).
func TestTimeoutConversions(t *testing.T) {
	if got := vl53l0xDecodeVcselPeriod(0x06); got != 14 {
		t.Errorf("vcsel(0x06) = %d, want 14", got)
	}
	if got := vl53l0xDecodeVcselPeriod(0x04); got != 10 {
		t.Errorf("vcsel(0x04) = %d, want 10", got)
	}
	mclksToUs := []struct {
		mclks float64
		vcsel int
		us    float64
	}{
		{38, 14, 2055},   // MSRC/DSS/TCC (register 0x25 -> 38 MCLKs)
		{151, 14, 8087},  // pre range
		{358, 10, 13669}, // final range minus pre range
	}
	for _, c := range mclksToUs {
		if got := vl53l0xTimeoutMclksToUs(c.mclks, c.vcsel); got != c.us {
			t.Errorf("mclksToUs(%v, %d) = %v, want %v", c.mclks, c.vcsel, got, c.us)
		}
	}
	if got := vl53l0xTimeoutUsToMclks(14259, 10); got != 374 {
		t.Errorf("usToMclks(14259, 10) = %v, want 374", got)
	}
}

func TestSequenceStepEnables(t *testing.T) {
	// 0xE8 is the sequence config the driver leaves active.
	e := vl53l0xSequenceStepEnables(0xE8)
	if e.tcc || !e.dss || e.msrc || !e.preRange || !e.finalRange {
		t.Errorf("enables(0xE8) = %+v, want dss/pre/final only", e)
	}
	e = vl53l0xSequenceStepEnables(0xFF)
	if !(e.tcc && e.dss && e.msrc && e.preRange && e.finalRange) {
		t.Errorf("enables(0xFF) = %+v, want all", e)
	}
}

// The timing budget the getter computes from the tuning-default registers,
// and the final range timeout the setter re-encodes for the same budget —
// golden values generated with the adafruit_vl53l0x reference driver's math.
func TestTimingBudgetMath(t *testing.T) {
	msrcDssTccUs := vl53l0xTimeoutMclksToUs(38, 14)
	preRangeUs := vl53l0xTimeoutMclksToUs(151, 14)
	finalRangeUs := vl53l0xTimeoutMclksToUs(509-151, 10)

	// Getter with sequence config 0xE8 (dss + pre range + final range).
	budgetUs := float64(1910+960) + 2*(msrcDssTccUs+690) + (preRangeUs + 660) + (finalRangeUs + 550)
	if budgetUs != 31326 {
		t.Errorf("timing budget = %v, want 31326", budgetUs)
	}

	// Setter with the same budget.
	usedBudgetUs := float64(1320+960) + 2*(msrcDssTccUs+690) + (preRangeUs + 660) + 550
	finalRangeTimeoutMclks := vl53l0xTimeoutUsToMclks(budgetUs-usedBudgetUs, 10) + 151
	if got := vl53l0xEncodeTimeout(finalRangeTimeoutMclks); got != 0x0283 {
		t.Errorf("re-encoded final range timeout = 0x%04X, want 0x0283", got)
	}
}

func TestMaskRefSpadMap(t *testing.T) {
	cases := []struct {
		name       string
		in         [6]byte
		count      int
		isAperture bool
		want       [6]byte
		enabled    int
	}{
		{
			// Aperture SPADs start at bit 12; the first 12 bits are cleared.
			"aperture", [6]byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}, 5, true,
			[6]byte{0x00, 0xF0, 0x01, 0x00, 0x00, 0x00}, 5,
		},
		{
			"non-aperture", [6]byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}, 3, false,
			[6]byte{0x07, 0x00, 0x00, 0x00, 0x00, 0x00}, 3,
		},
		{
			// Holes in the good SPAD map are skipped, not counted.
			"holes", [6]byte{0x00, 0xCC, 0x00, 0x00, 0x00, 0x00}, 2, true,
			[6]byte{0x00, 0xC0, 0x00, 0x00, 0x00, 0x00}, 2,
		},
		{
			// Asking for more SPADs than exist enables all remaining ones.
			"all", [6]byte{0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}, 48, true,
			[6]byte{0x00, 0xF0, 0xFF, 0xFF, 0xFF, 0xFF}, 36,
		},
	}
	for _, c := range cases {
		m := c.in
		enabled := vl53l0xMaskRefSpadMap(&m, c.count, c.isAperture)
		if m != c.want {
			t.Errorf("%s: map = %02X, want %02X", c.name, m, c.want)
		}
		if enabled != c.enabled {
			t.Errorf("%s: enabled = %d, want %d", c.name, enabled, c.enabled)
		}
	}
}
