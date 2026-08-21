#!/usr/bin/env node
'use strict';
/*
 * Unit test: the raw-word → °C conversion must match the Python node's
 * decoding — 12-bit magnitude in 1/16 °C, bit 12 sign (two's complement),
 * alert flag bits 15..13 ignored.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { decodeTemperature } = require('./mcp9808_driver');

const cases = [
  [0x0000, 0.0],
  [0x0001, 0.0625],   // one LSB
  [0x0194, 25.25],    // MCP9808 datasheet example (+25.25 °C)
  [0x0fff, 255.9375], // largest positive magnitude
  [0x1fff, -0.0625],  // -1 LSB (two's complement)
  [0x1fe8, -1.5],
  [0x1e70, -25.0],
  [0x1000, -256.0],
  [0xe194, 25.25],    // alert flag bits 15..13 must be masked off
];

for (const [word, want] of cases) {
  const got = decodeTemperature(word);
  assert.strictEqual(got, want, `decodeTemperature(0x${word.toString(16)}) = ${got} != ${want}`);
}

console.log(`OK: ${cases.length} raw-word -> °C conversions match the Python node`);
