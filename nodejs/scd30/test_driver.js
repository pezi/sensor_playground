#!/usr/bin/env node
'use strict';
/*
 * Unit tests for the portable SCD30 driver math: the Sensirion CRC-8
 * (datasheet example 0xBE 0xEF -> 0x92), the per-word CRC frame parsing
 * and the big-endian word-pair IEEE-754 float decoding, checked against
 * the example measurement frame from the SCD30 interface description
 * (439.09 ppm CO2, 27.2 °C, 48.8 %RH).
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { crc8, parseWords, decodeFloat, decodeMeasurement } = require('./scd30_driver');

// The read-measurement example frame from the Sensirion SCD30 interface
// description: CO2 0x43DB8C2E, temperature 0x41D9E7FF, humidity 0x42433A1B.
const exampleFrame = Buffer.from([
  0x43, 0xdb, 0xcb, 0x8c, 0x2e, 0x8f, // CO2 = 439.095 ppm
  0x41, 0xd9, 0x70, 0xe7, 0xff, 0xf5, // temperature = 27.238 °C
  0x42, 0x43, 0xbf, 0x3a, 0x1b, 0x74, // humidity = 48.806 %RH
]);

// CRC-8 known vectors.
const crcCases = [
  [[0xbe, 0xef], 0x92], // Sensirion datasheet example
  [[0x00, 0x00], 0x81], // start-periodic argument (pressure 0)
  [[0x00, 0x02], 0xe3], // set-interval argument (2 s)
  [[0x43, 0xdb], 0xcb],
  [[0x8c, 0x2e], 0x8f],
];
for (const [data, want] of crcCases) {
  assert.strictEqual(crc8(data), want, `crc8(${data}) != 0x${want.toString(16)}`);
}

// Frame parsing: valid, corrupt CRC, short frame.
const words = parseWords(exampleFrame);
assert.ok(words !== null, 'valid frame rejected');
assert.strictEqual(words.length, 6);
assert.strictEqual(words[0], 0x43db);
assert.strictEqual(words[1], 0x8c2e);

const corrupt = Buffer.from(exampleFrame);
corrupt[5] ^= 0x01;
assert.strictEqual(parseWords(corrupt), null, 'corrupt CRC accepted');
assert.strictEqual(parseWords(exampleFrame.slice(0, 4)), null, 'short frame accepted');

// Float decoding: the words are combined MSW-first into a big-endian
// IEEE-754 single.
assert.strictEqual(decodeFloat(0x3f80, 0x0000), 1.0);
assert.strictEqual(decodeFloat(0x0000, 0x0000), 0.0);

const m = decodeMeasurement(exampleFrame);
assert.ok(m !== null, 'valid measurement rejected');
assert.ok(Math.abs(m.co2 - 439.09515380859375) < 1e-9, `co2 ${m.co2}`);
assert.ok(Math.abs(m.temperature - 27.238279342651367) < 1e-9, `temperature ${m.temperature}`);
assert.ok(Math.abs(m.humidity - 48.80674362182617) < 1e-9, `humidity ${m.humidity}`);
assert.strictEqual(decodeMeasurement(corrupt), null, 'corrupt measurement accepted');

console.log('OK: CRC-8, frame parsing and float decoding match the SCD30 datasheet');
