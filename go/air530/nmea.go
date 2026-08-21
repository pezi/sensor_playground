// NMEA-0183 parsing for the Air530 GPS — a port of the Python node's
// parser (itself a port of dart_periphery's NmeaParser, see
// serial_air530.dart):
//
//   - GGA sentences (preferred): latitude, longitude, MSL altitude,
//     satellites in use
//   - GLL sentences (fallback): latitude, longitude only
//   - Sentences with bad checksums are skipped
package main

import (
	"fmt"
	"math"
	"strconv"
	"strings"
)

func round6(x float64) float64 { return math.Round(x*1e6) / 1e6 }

// isDigits reports whether s is non-empty and all ASCII digits
// (Python's str.isdigit for the values NMEA carries).
func isDigits(s string) bool {
	if s == "" {
		return false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

// checksumOK verifies the XOR checksum between '$' and '*'.
func checksumOK(line string) bool {
	star := strings.LastIndex(line, "*")
	if star <= 0 || star+3 > len(line) {
		return false
	}
	data := line[1:star]
	given := line[star+1:]
	var calc byte
	for i := 0; i < len(data); i++ {
		calc ^= data[i]
	}
	return fmt.Sprintf("%02X", calc) == strings.ToUpper(given)
}

// coordToDecimal converts NMEA ddmm.mmmm / dddmm.mmmm + hemisphere to
// decimal degrees (ok=false when the field is empty or malformed).
func coordToDecimal(coord, hemi string) (float64, bool) {
	if coord == "" || hemi == "" || !strings.Contains(coord, ".") {
		return 0, false
	}
	// Latitude has 2 degree digits, longitude 3 — infer from the hemisphere.
	degLen := 2
	if hemi == "E" || hemi == "W" {
		degLen = 3
	}
	if len(coord) < degLen {
		return 0, false
	}
	degrees, err := strconv.ParseFloat(coord[:degLen], 64)
	if err != nil {
		return 0, false
	}
	minutes, err := strconv.ParseFloat(coord[degLen:], 64)
	if err != nil {
		return 0, false
	}
	decimal := degrees + minutes/60.0
	if hemi == "S" || hemi == "W" {
		decimal = -decimal
	}
	return decimal, true
}

// parseGGA parses a GGA sentence: lat, lon, MSL altitude, satellites in use.
func parseGGA(fields []string) map[string]any {
	if len(fields) < 10 {
		return nil
	}
	fixQuality := fields[6]
	if !isDigits(fixQuality) {
		return nil
	}
	if q, _ := strconv.Atoi(fixQuality); q == 0 {
		return nil
	}
	lat, latOK := coordToDecimal(fields[2], fields[3])
	lon, lonOK := coordToDecimal(fields[4], fields[5])
	if !latOK || !lonOK {
		return nil
	}
	fix := map[string]any{"latitude": round6(lat), "longitude": round6(lon)}
	if altitude, err := strconv.ParseFloat(fields[9], 64); err == nil {
		fix["altitude"] = round1(altitude)
	}
	if isDigits(fields[7]) {
		sats, _ := strconv.Atoi(fields[7])
		fix["satellites"] = sats
	}
	return fix
}

// parseGLL parses a GLL sentence: lat and lon only.
func parseGLL(fields []string) map[string]any {
	if len(fields) < 7 || !strings.HasPrefix(fields[6], "A") {
		return nil
	}
	lat, latOK := coordToDecimal(fields[1], fields[2])
	lon, lonOK := coordToDecimal(fields[3], fields[4])
	if !latOK || !lonOK {
		return nil
	}
	return map[string]any{"latitude": round6(lat), "longitude": round6(lon)}
}

// parseNMEA parses a multi-line NMEA block and returns the first valid
// fix, or nil. GGA fixes (with altitude and satellite count) are
// preferred over GLL.
func parseNMEA(block string) map[string]any {
	var ggaFix, gllFix map[string]any
	for _, raw := range strings.Split(block, "\n") {
		line := strings.TrimSpace(raw)
		if !strings.HasPrefix(line, "$") || !strings.Contains(line, "*") {
			continue
		}
		if !checksumOK(line) {
			continue
		}
		fields := strings.Split(line[1:strings.Index(line, "*")], ",")
		sentence := fields[0]
		if len(sentence) >= 5 {
			sentence = sentence[len(sentence)-3:]
		}
		if sentence == "GGA" && ggaFix == nil {
			ggaFix = parseGGA(fields)
		} else if sentence == "GLL" && gllFix == nil {
			gllFix = parseGLL(fields)
		}
	}
	if ggaFix != nil {
		return ggaFix
	}
	return gllFix
}
