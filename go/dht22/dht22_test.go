package main

import (
	"math"
	"testing"
)

func withChecksum(b0, b1, b2, b3 byte) [5]byte {
	return [5]byte{b0, b1, b2, b3, byte(int(b0) + int(b1) + int(b2) + int(b3))}
}

func fallingTrain(buf [5]byte) []uint64 {
	ts := uint64(5_000_000)
	train := []uint64{ts}
	ts += 160_000
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

func TestDecodeFrameDHT22(t *testing.T) {
	want := withChecksum(0x02, 0x8C, 0x01, 0x01) // 65.2 %RH, 25.7 °C
	got, err := decodeFrame(fallingTrain(want))
	if err != nil {
		t.Fatalf("decodeFrame: %v", err)
	}
	if got != want {
		t.Fatalf("decodeFrame = % x, want % x", got, want)
	}
	temperature, humidity := convertDHT22(got)
	if math.Abs(temperature-25.7) > 1e-9 || math.Abs(humidity-65.2) > 1e-9 {
		t.Errorf("convertDHT22 = (%v, %v), want (25.7, 65.2)", temperature, humidity)
	}
}

func TestConvertDHT22NegativeTemperature(t *testing.T) {
	buf := withChecksum(0x01, 0x90, 0x80, 0x65) // 40.0 %RH, -10.1 °C
	temperature, humidity := convertDHT22(buf)
	if math.Abs(temperature-(-10.1)) > 1e-9 || math.Abs(humidity-40.0) > 1e-9 {
		t.Errorf("convertDHT22 = (%v, %v), want (-10.1, 40)", temperature, humidity)
	}
}

func TestDecodeFrameTrimsLeadingEdges(t *testing.T) {
	want := withChecksum(0x02, 0x8C, 0x00, 0xFA)
	train := append([]uint64{4_800_000, 4_900_000}, fallingTrain(want)...)
	got, err := decodeFrame(train)
	if err != nil {
		t.Fatalf("decodeFrame: %v", err)
	}
	if got != want {
		t.Fatalf("decodeFrame = % x, want % x", got, want)
	}
}

func TestDecodeFrameRejectsBadInput(t *testing.T) {
	buf := withChecksum(0x02, 0x8C, 0x01, 0x01)
	corrupt := buf
	corrupt[4]++
	if _, err := decodeFrame(fallingTrain(corrupt)); err == nil {
		t.Error("decodeFrame accepted a frame with a bad checksum")
	}
	if _, err := decodeFrame(fallingTrain(buf)[:40]); err == nil {
		t.Error("decodeFrame accepted a truncated frame")
	}
}

func TestEmulationUsesDHT22Precision(t *testing.T) {
	got := emulatedReading(60, 0)
	wantTemperature := round1(22.0 + 2.0*math.Sin(1))
	wantHumidity := round1(45.0 + 8.0*math.Sin(60.0/97.0))
	if got["temperature"] != wantTemperature || got["humidity"] != wantHumidity {
		t.Fatalf("emulatedReading = %v, want temperature=%v humidity=%v", got, wantTemperature, wantHumidity)
	}
}
