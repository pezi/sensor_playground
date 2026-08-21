//! NMEA-0183 parsing for the Air530 GPS — a port of the Python node's
//! parser (itself a port of dart_periphery's NmeaParser, see
//! serial_air530.dart):
//!
//! - GGA sentences (preferred): latitude, longitude, MSL altitude,
//!   satellites in use
//! - GLL sentences (fallback): latitude, longitude only
//! - Sentences with bad checksums are skipped

use common::{round1, Payload};
use serde_json::json;

/// Round like the Python node's round(x, 6) for the coordinates.
pub fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

/// Python's str.isdigit for the values NMEA carries.
fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Verify the XOR checksum between '$' and '*'.
fn checksum_ok(line: &str) -> bool {
    let Some(star) = line.rfind('*') else {
        return false;
    };
    if star == 0 || star + 3 > line.len() {
        return false;
    }
    let calc = line[1..star].bytes().fold(0u8, |a, b| a ^ b);
    format!("{calc:02X}") == line[star + 1..].to_uppercase()
}

/// Convert NMEA ddmm.mmmm / dddmm.mmmm + hemisphere to decimal degrees.
fn coord_to_decimal(coord: &str, hemi: &str) -> Option<f64> {
    if coord.is_empty() || hemi.is_empty() || !coord.contains('.') {
        return None;
    }
    // Latitude has 2 degree digits, longitude 3 — infer from the hemisphere.
    let deg_len = if hemi == "E" || hemi == "W" { 3 } else { 2 };
    if coord.len() < deg_len {
        return None;
    }
    let degrees: f64 = coord[..deg_len].parse().ok()?;
    let minutes: f64 = coord[deg_len..].parse().ok()?;
    let decimal = degrees + minutes / 60.0;
    Some(if hemi == "S" || hemi == "W" { -decimal } else { decimal })
}

/// Parse a GGA sentence: lat, lon, MSL altitude, satellites in use.
fn parse_gga(fields: &[&str]) -> Option<Payload> {
    if fields.len() < 10 {
        return None;
    }
    let fix_quality = fields[6];
    if !is_digits(fix_quality) || fix_quality.parse::<u32>().ok()? == 0 {
        return None;
    }
    let lat = coord_to_decimal(fields[2], fields[3])?;
    let lon = coord_to_decimal(fields[4], fields[5])?;
    let mut fix = Payload::new();
    fix.insert("latitude".into(), json!(round6(lat)));
    fix.insert("longitude".into(), json!(round6(lon)));
    if let Ok(altitude) = fields[9].parse::<f64>() {
        fix.insert("altitude".into(), json!(round1(altitude)));
    }
    if is_digits(fields[7]) {
        if let Ok(sats) = fields[7].parse::<u32>() {
            fix.insert("satellites".into(), json!(sats));
        }
    }
    Some(fix)
}

/// Parse a GLL sentence: lat and lon only.
fn parse_gll(fields: &[&str]) -> Option<Payload> {
    if fields.len() < 7 || !fields[6].starts_with('A') {
        return None;
    }
    let lat = coord_to_decimal(fields[1], fields[2])?;
    let lon = coord_to_decimal(fields[3], fields[4])?;
    let mut fix = Payload::new();
    fix.insert("latitude".into(), json!(round6(lat)));
    fix.insert("longitude".into(), json!(round6(lon)));
    Some(fix)
}

/// Parse a multi-line NMEA block; return the first valid fix or None.
/// GGA fixes (with altitude and satellite count) are preferred over GLL.
pub fn parse_nmea(block: &str) -> Option<Payload> {
    let mut gga_fix = None;
    let mut gll_fix = None;
    for raw in block.lines() {
        let line = raw.trim();
        if !line.starts_with('$') || !line.contains('*') {
            continue;
        }
        if !checksum_ok(line) {
            continue;
        }
        let star = line.find('*').unwrap();
        let fields: Vec<&str> = line[1..star].split(',').collect();
        let sentence = if fields[0].len() >= 5 {
            &fields[0][fields[0].len() - 3..]
        } else {
            fields[0]
        };
        if sentence == "GGA" && gga_fix.is_none() {
            gga_fix = parse_gga(&fields);
        } else if sentence == "GLL" && gll_fix.is_none() {
            gll_fix = parse_gll(&fields);
        }
    }
    gga_fix.or(gll_fix)
}

// The NMEA parser is checked against golden values generated with the
// Python node's parser.
#[cfg(test)]
mod tests {
    use super::*;

    // Reference sentences with valid checksums.
    const GGA_FIX: &str = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47";
    const GGA_NO_FIX: &str = "$GPGGA,123519,4807.038,N,01131.000,E,0,00,,,M,,M,,*52";
    const GLL_WEST: &str = "$GPGLL,4916.45,N,12311.12,W,225444,A,*1D";
    const GLL_SOUTH: &str = "$GNGLL,3751.65,S,14507.36,E,225444,A,*05";

    #[test]
    fn parses_gga_fix() {
        let fix = parse_nmea(GGA_FIX).unwrap();
        assert_eq!(fix["latitude"], 48.1173);
        assert_eq!(fix["longitude"], 11.516667);
        assert_eq!(fix["altitude"], 545.4);
        assert_eq!(fix["satellites"], 8);
    }

    #[test]
    fn parses_gll_hemispheres() {
        let fix = parse_nmea(GLL_WEST).unwrap();
        assert_eq!(fix["latitude"], 49.274167);
        assert_eq!(fix["longitude"], -123.185333);
        assert!(!fix.contains_key("altitude"));

        let fix = parse_nmea(GLL_SOUTH).unwrap();
        assert_eq!(fix["latitude"], -37.860833);
        assert_eq!(fix["longitude"], 145.122667);
    }

    #[test]
    fn prefers_gga_over_gll() {
        let block = format!("{GLL_WEST}\r\n{GGA_FIX}\r\n");
        let fix = parse_nmea(&block).unwrap();
        assert_eq!(fix["latitude"], 48.1173);
        assert_eq!(fix["satellites"], 8);
    }

    #[test]
    fn no_fix_lines() {
        // Fix quality 0 -> no fix; a following GLL still counts.
        assert!(parse_nmea(GGA_NO_FIX).is_none());
        let fix = parse_nmea(&format!("{GGA_NO_FIX}\n{GLL_WEST}")).unwrap();
        assert_eq!(fix["latitude"], 49.274167);
    }

    #[test]
    fn bad_checksum_skipped() {
        let bad = "$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*00";
        assert!(parse_nmea(bad).is_none());
        // A partial first line (burst starts mid-sentence) is skipped too.
        let block = format!("31.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n{GGA_FIX}");
        assert_eq!(parse_nmea(&block).unwrap()["latitude"], 48.1173);
        // Lower-case checksum digits are accepted.
        assert!(checksum_ok("$GPGLL,4916.45,N,12311.12,W,225444,A,*1d"));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_nmea("").is_none());
        assert!(parse_nmea("no nmea here\r\n$*\r\n$GPGGA\r\n").is_none());
        assert!(coord_to_decimal("", "N").is_none());
        assert!(coord_to_decimal("4807.038", "").is_none());
        assert!(coord_to_decimal("4807", "N").is_none()); // no decimal point
    }
}
