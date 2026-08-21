'use strict';
/*
 * NMEA-0183 parsing for the Air530 GPS — a port of the Python node's
 * parser (itself a port of dart_periphery's NmeaParser, see
 * serial_air530.dart):
 *
 * - GGA sentences (preferred): latitude, longitude, MSL altitude,
 *   satellites in use
 * - GLL sentences (fallback): latitude, longitude only
 * - Sentences with bad checksums are skipped
 */

const round1 = (x) => Math.round(x * 10) / 10;
const round6 = (x) => Math.round(x * 1e6) / 1e6;

/** Python's str.isdigit for the values NMEA carries. */
const isDigits = (s) => /^[0-9]+$/.test(s || '');

/** Verify the XOR checksum between '$' and '*'. */
function checksumOk(line) {
  const star = line.lastIndexOf('*');
  if (star <= 0 || star + 3 > line.length) return false;
  const given = line.slice(star + 1);
  let calc = 0;
  for (let i = 1; i < star; i++) calc ^= line.charCodeAt(i);
  return calc.toString(16).toUpperCase().padStart(2, '0') === given.toUpperCase();
}

/** Convert NMEA ddmm.mmmm / dddmm.mmmm + hemisphere to decimal degrees. */
function coordToDecimal(coord, hemi) {
  if (!coord || !hemi || !coord.includes('.')) return null;
  // Latitude has 2 degree digits, longitude 3 — infer from the hemisphere.
  const degLen = hemi === 'E' || hemi === 'W' ? 3 : 2;
  const degrees = Number(coord.slice(0, degLen));
  const minutes = Number(coord.slice(degLen));
  if (!Number.isFinite(degrees) || !Number.isFinite(minutes)) return null;
  const decimal = degrees + minutes / 60.0;
  return hemi === 'S' || hemi === 'W' ? -decimal : decimal;
}

/** Parse a GGA sentence: lat, lon, MSL altitude, satellites in use. */
function parseGga(fields) {
  if (fields.length < 10) return null;
  const fixQuality = fields[6];
  if (!isDigits(fixQuality) || parseInt(fixQuality, 10) === 0) return null;
  const lat = coordToDecimal(fields[2], fields[3]);
  const lon = coordToDecimal(fields[4], fields[5]);
  if (lat === null || lon === null) return null;
  const fix = { latitude: round6(lat), longitude: round6(lon) };
  const altitude = Number(fields[9]);
  if (fields[9] !== '' && Number.isFinite(altitude)) fix.altitude = round1(altitude);
  if (isDigits(fields[7])) fix.satellites = parseInt(fields[7], 10);
  return fix;
}

/** Parse a GLL sentence: lat and lon only. */
function parseGll(fields) {
  if (fields.length < 7 || !fields[6].startsWith('A')) return null;
  const lat = coordToDecimal(fields[1], fields[2]);
  const lon = coordToDecimal(fields[3], fields[4]);
  if (lat === null || lon === null) return null;
  return { latitude: round6(lat), longitude: round6(lon) };
}

/*
 * Parse a multi-line NMEA block; return the first valid fix or null.
 * GGA fixes (with altitude and satellite count) are preferred over GLL.
 */
function parseNmea(block) {
  let ggaFix = null;
  let gllFix = null;
  for (const raw of (block || '').split(/\r?\n/)) {
    const line = raw.trim();
    if (!line.startsWith('$') || !line.includes('*')) continue;
    if (!checksumOk(line)) continue;
    const fields = line.slice(1, line.indexOf('*')).split(',');
    const sentence = fields[0].length >= 5 ? fields[0].slice(-3) : fields[0];
    if (sentence === 'GGA' && ggaFix === null) {
      ggaFix = parseGga(fields);
    } else if (sentence === 'GLL' && gllFix === null) {
      gllFix = parseGll(fields);
    }
  }
  return ggaFix || gllFix;
}

module.exports = { checksumOk, coordToDecimal, parseNmea };
