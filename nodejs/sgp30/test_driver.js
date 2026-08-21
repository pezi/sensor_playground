#!/usr/bin/env node
'use strict';
/*
 * The CRC-8 and response parsing are checked against the SGP30 datasheet
 * example (0xBEEF -> 0x92, section 6.6) and vectors generated with the
 * Pimoroni sgp30-python reference driver.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { calculateCrc, parseWords } = require('./sgp30_driver');

const crcCases = [
  [0x0000, 0x81],
  [0xbeef, 0x92], // datasheet example
  [0x1234, 0x37],
  [0x0190, 0x4c], // 400 ppm, the warm-up eCO2 value
  [0x8000, 0xa2],
  [0xffff, 0xac],
];
for (const [word, crc] of crcCases) {
  assert.strictEqual(calculateCrc(word), crc,
    `calculateCrc(0x${word.toString(16)}) != 0x${crc.toString(16)}`);
}

// A valid measure_air_quality warm-up response: 400 ppm, 0 ppb.
assert.deepStrictEqual(
  parseWords(Buffer.from([0x01, 0x90, 0x4c, 0x00, 0x00, 0x81])),
  [400, 0]
);

// A corrupted CRC must be rejected.
assert.throws(() => parseWords(Buffer.from([0x01, 0x90, 0x4d])), /invalid CRC/);

console.log(`OK: ${crcCases.length} CRC vectors and response parsing match the reference driver`);
