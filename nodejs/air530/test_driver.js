#!/usr/bin/env node
'use strict';
/*
 * The NMEA parser is checked against golden values generated with the
 * Python node's parser (itself a port of dart_periphery's NmeaParser).
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { checksumOk, coordToDecimal, parseNmea } = require('./air530_driver');

// Reference sentences with valid checksums.
const GGA_FIX = '$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*47';
const GGA_NO_FIX = '$GPGGA,123519,4807.038,N,01131.000,E,0,00,,,M,,M,,*52';
const GLL_WEST = '$GPGLL,4916.45,N,12311.12,W,225444,A,*1D';
const GLL_SOUTH = '$GNGLL,3751.65,S,14507.36,E,225444,A,*05';

// GGA: full fix with altitude and satellite count.
assert.deepStrictEqual(parseNmea(GGA_FIX), {
  latitude: 48.1173,
  longitude: 11.516667,
  altitude: 545.4,
  satellites: 8,
});

// GLL: hemispheres map to signed decimal degrees.
assert.deepStrictEqual(parseNmea(GLL_WEST), { latitude: 49.274167, longitude: -123.185333 });
assert.deepStrictEqual(parseNmea(GLL_SOUTH), { latitude: -37.860833, longitude: 145.122667 });

// GGA fixes are preferred over GLL, whatever the line order.
assert.strictEqual(parseNmea(`${GLL_WEST}\r\n${GGA_FIX}\r\n`).satellites, 8);

// Fix quality 0 -> no fix; a following GLL still counts.
assert.strictEqual(parseNmea(GGA_NO_FIX), null);
assert.strictEqual(parseNmea(`${GGA_NO_FIX}\n${GLL_WEST}`).latitude, 49.274167);

// Bad checksums and partial first lines (burst starts mid-sentence) are
// skipped.
assert.strictEqual(
  parseNmea('$GPGGA,123519,4807.038,N,01131.000,E,1,08,0.9,545.4,M,46.9,M,,*00'),
  null
);
assert.strictEqual(parseNmea(`31.000,E,1,08,0.9,545.4,M,46.9,M,,*47\r\n${GGA_FIX}`).latitude, 48.1173);
assert.strictEqual(checksumOk('$GPGLL,4916.45,N,12311.12,W,225444,A,*1d'), true); // case-insensitive

// Garbage and empty blocks.
assert.strictEqual(parseNmea(''), null);
assert.strictEqual(parseNmea('no nmea here\r\n$*\r\n$GPGGA\r\n'), null);
assert.strictEqual(coordToDecimal('', 'N'), null);
assert.strictEqual(coordToDecimal('4807.038', ''), null);
assert.strictEqual(coordToDecimal('4807', 'N'), null); // no decimal point

console.log('OK: NMEA parser matches the Python node golden values');
