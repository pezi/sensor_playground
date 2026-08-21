// The raw-to-value conversion is checked against golden values computed
// with the Python node's formulas (20-bit humidity / 20-bit temperature
// split across the shared nibble in byte 3). The AHT10/AHT20 protocol the
// Python node uses reads 6 bytes and has no CRC.
package main

import (
	"math"
	"sync"
	"testing"
)

type transactionCheckingBus struct {
	mu           sync.Mutex
	inProgress   bool
	overlapped   bool
	transactions int
}

func (b *transactionCheckingBus) Write(_ []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.inProgress {
		b.overlapped = true
	}
	b.inProgress = true
	return len(ahtCmdMeasure), nil
}

func (b *transactionCheckingBus) Read(raw []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	copy(raw, []byte{0x1C, 0x80, 0x00, 0x08, 0x00, 0x00})
	b.inProgress = false
	b.transactions++
	return len(raw), nil
}

func (b *transactionCheckingBus) Close() error { return nil }

func TestParseAHT20GoldenValues(t *testing.T) {
	cases := []struct {
		raw  []byte
		temp float64
		hum  float64
	}{
		// All-zero raw values -> scale minimum.
		{[]byte{0x1C, 0x00, 0x00, 0x00, 0x00, 0x00}, -50.0, 0.0},
		// Mid-scale on both channels (0x80000 of 0x100000).
		{[]byte{0x1C, 0x80, 0x00, 0x08, 0x00, 0x00}, 50.0, 50.0},
		// All-ones raw values -> scale maximum.
		{[]byte{0x1C, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF}, 149.99980926513672, 99.99990463256836},
		// Realistic indoor reading, shared nibble in byte 3 non-zero.
		{[]byte{0x1C, 0x6E, 0x14, 0x85, 0xC7, 0x2A}, 22.224807739257812, 43.000030517578125},
		// AHT10-style status byte (calibrated bit only).
		{[]byte{0x08, 0x5D, 0x2E, 0x95, 0x8C, 0x51}, 19.35138702392578, 36.399173736572266},
	}
	for i, c := range cases {
		temp, hum, ok := parseAHT20(c.raw)
		if !ok {
			t.Fatalf("case %d: unexpected busy", i)
		}
		if math.Abs(temp-c.temp) > 1e-12 {
			t.Fatalf("case %d: temperature %v != golden %v", i, temp, c.temp)
		}
		if math.Abs(hum-c.hum) > 1e-12 {
			t.Fatalf("case %d: humidity %v != golden %v", i, hum, c.hum)
		}
	}
}

// A set busy bit (0x80 in the status byte) must yield no reading, like
// the Python node returning None.
func TestParseAHT20Busy(t *testing.T) {
	if _, _, ok := parseAHT20([]byte{0x80, 0x12, 0x34, 0x56, 0x78, 0x9A}); ok {
		t.Fatal("busy status byte must not produce a reading")
	}
}

func TestReadSerializesSensorTransactions(t *testing.T) {
	bus := &transactionCheckingBus{}
	sensor := &AHT20{f: bus}

	start := make(chan struct{})
	var wg sync.WaitGroup
	for range 2 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			<-start
			if _, err := sensor.Read(); err != nil {
				t.Errorf("Read failed: %v", err)
			}
		}()
	}
	close(start)
	wg.Wait()

	bus.mu.Lock()
	defer bus.mu.Unlock()
	if bus.overlapped {
		t.Fatal("AHT20 trigger/wait/read transactions overlapped")
	}
	if bus.transactions != 2 {
		t.Fatalf("completed %d transactions, want 2", bus.transactions)
	}
}
