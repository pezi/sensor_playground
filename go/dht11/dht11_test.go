package main

import (
	"math"
	"testing"
)

// withChecksum completes a 4-byte payload with the sensor's checksum
// (sum of the four bytes, low byte).
func withChecksum(b0, b1, b2, b3 byte) [5]byte {
	return [5]byte{b0, b1, b2, b3, byte(int(b0) + int(b1) + int(b2) + int(b3))}
}

// fallingTrain builds the kernel-timestamped falling edges of one frame:
// the response preamble (80 µs low + 80 µs high), then 40 bit slots of
// 50 µs low plus ~28 µs (0) or ~70 µs (1) high, closed by the sensor's
// final low pull — 42 falling edges, ~78 µs falling-to-falling for a 0
// and ~120 µs for a 1.
func fallingTrain(buf [5]byte) []uint64 {
	ts := uint64(5_000_000) // arbitrary start of the response preamble
	train := []uint64{ts}
	ts += 160_000 // preamble: 80 µs low + 80 µs high
	train = append(train, ts)
	for i := 0; i < 40; i++ {
		if buf[i/8]&(1<<(7-i%8)) != 0 {
			ts += 120_000
		} else {
			ts += 78_000
		}
		train = append(train, ts)
	}
	return train
}

func TestDecodeFrameDHT11(t *testing.T) {
	want := withChecksum(45, 0, 22, 0) // 45 %RH, 22 °C
	got, err := decodeFrame(fallingTrain(want))
	if err != nil {
		t.Fatalf("decodeFrame: %v", err)
	}
	if got != want {
		t.Fatalf("decodeFrame = % x, want % x", got, want)
	}
	temperature, humidity := convert(got, false)
	if temperature != 22.0 || humidity != 45.0 {
		t.Errorf("convert = (%v, %v), want (22, 45)", temperature, humidity)
	}
}

func TestDecodeFrameDHT22(t *testing.T) {
	want := withChecksum(0x02, 0x8C, 0x01, 0x01) // 652 -> 65.2 %RH, 257 -> 25.7 °C
	got, err := decodeFrame(fallingTrain(want))
	if err != nil {
		t.Fatalf("decodeFrame: %v", err)
	}
	if got != want {
		t.Fatalf("decodeFrame = % x, want % x", got, want)
	}
	temperature, humidity := convert(got, true)
	if math.Abs(temperature-25.7) > 1e-9 || math.Abs(humidity-65.2) > 1e-9 {
		t.Errorf("convert = (%v, %v), want (25.7, 65.2)", temperature, humidity)
	}
}

// The DHT22 sends negative temperatures as sign bit + magnitude, not
// two's complement.
func TestConvertDHT22NegativeTemperature(t *testing.T) {
	buf := withChecksum(0x01, 0x90, 0x80, 0x65) // 40.0 %RH, -10.1 °C
	temperature, humidity := convert(buf, true)
	if math.Abs(temperature-(-10.1)) > 1e-9 || math.Abs(humidity-40.0) > 1e-9 {
		t.Errorf("convert = (%v, %v), want (-10.1, 40)", temperature, humidity)
	}
}

// Extra edges before the frame — a partially caught preamble, or noise
// from the host releasing the line — must not shift the bits: the decoder
// anchors on the last 41 falling edges.
func TestDecodeFrameTrimsLeadingEdges(t *testing.T) {
	want := withChecksum(0x02, 0x8C, 0x00, 0xFA) // 65.2 %RH, 25.0 °C
	train := append([]uint64{4_800_000, 4_900_000}, fallingTrain(want)...)
	got, err := decodeFrame(train)
	if err != nil {
		t.Fatalf("decodeFrame: %v", err)
	}
	if got != want {
		t.Fatalf("decodeFrame = % x, want % x", got, want)
	}
}

func TestDecodeFrameChecksumMismatch(t *testing.T) {
	buf := withChecksum(45, 0, 22, 0)
	buf[4]++ // corrupt the checksum byte
	if _, err := decodeFrame(fallingTrain(buf)); err == nil {
		t.Error("decodeFrame accepted a frame with a bad checksum")
	}
}

func TestDecodeFrameIncomplete(t *testing.T) {
	train := fallingTrain(withChecksum(45, 0, 22, 0))
	if _, err := decodeFrame(train[:40]); err == nil {
		t.Error("decodeFrame accepted a truncated bit train")
	}
}
