// The NMEA parser is checked against golden values generated with the
// Python node's parser (itself a port of dart_periphery's NmeaParser).
package main

import (
	"reflect"
	"testing"
)

// Reference sentences with valid checksums.
const (
	ggaFix   = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47"
	ggaNoFix = "$GPGGA,123519,4807.038,N,01131.000,E,0,00,,,M,,M,,*52"
	gllWest  = "$GPGLL,4916.45,N,12311.12,W,225444,A,*1D"
	gllSouth = "$GNGLL,3751.65,S,14507.36,E,225444,A,*05"
)

func TestParsesGGAFix(t *testing.T) {
	want := map[string]any{
		"latitude":   48.1173,
		"longitude":  11.516667,
		"altitude":   545.4,
		"satellites": 8,
	}
	if got := parseNMEA(ggaFix); !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v, want %v", got, want)
	}
}

func TestParsesGLLHemispheres(t *testing.T) {
	want := map[string]any{"latitude": 49.274167, "longitude": -123.185333}
	if got := parseNMEA(gllWest); !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v, want %v", got, want)
	}
	want = map[string]any{"latitude": -37.860833, "longitude": 145.122667}
	if got := parseNMEA(gllSouth); !reflect.DeepEqual(got, want) {
		t.Fatalf("got %v, want %v", got, want)
	}
}

func TestPrefersGGAOverGLL(t *testing.T) {
	block := gllWest + "\r\n" + ggaFix + "\r\n"
	got := parseNMEA(block)
	if got == nil || got["latitude"] != 48.1173 || got["satellites"] != 8 {
		t.Fatalf("GGA fix not preferred: %v", got)
	}
}

func TestNoFixLines(t *testing.T) {
	// Fix quality 0 -> no fix; a following GLL still counts.
	if got := parseNMEA(ggaNoFix); got != nil {
		t.Fatalf("expected nil for fixless GGA, got %v", got)
	}
	got := parseNMEA(ggaNoFix + "\n" + gllWest)
	if got == nil || got["latitude"] != 49.274167 {
		t.Fatalf("GLL fallback failed: %v", got)
	}
}

func TestBadChecksumSkipped(t *testing.T) {
	bad := "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*00"
	if got := parseNMEA(bad); got != nil {
		t.Fatalf("expected nil for bad checksum, got %v", got)
	}
	// A partial first line (burst starts mid-sentence) is skipped too.
	got := parseNMEA("31.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n" + ggaFix)
	if got == nil || got["latitude"] != 48.1173 {
		t.Fatalf("partial line not skipped: %v", got)
	}
}

func TestGarbageAndEmpty(t *testing.T) {
	if got := parseNMEA(""); got != nil {
		t.Fatalf("expected nil for empty block, got %v", got)
	}
	if got := parseNMEA("no nmea here\r\n$*\r\n$GPGGA\r\n"); got != nil {
		t.Fatalf("expected nil for garbage, got %v", got)
	}
}
