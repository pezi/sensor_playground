#!/usr/bin/env node
'use strict';
/*
 * Unit test: the raw RAM word → °C conversion must match the Python node's
 * decoding — raw * 0.02 K - 273.15, with bit 15 marking an error rather
 * than a temperature.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { decodeTemperature } = require('./mlx90615_driver');

const cases = [
  [15000, 26.85],    // 15000 * 0.02 K = 300.00 K
  [14683, 20.51],    // room temperature
  [0x0000, -273.15], // 0 K — what an empty bus reads back
  [0x7fff, 382.19],  // largest value without the error flag
];

for (const [raw, want] of cases) {
  const got = decodeTemperature(raw);
  assert.ok(
    got !== null && Math.abs(got - want) < 1e-9,
    `decodeTemperature(${raw}) = ${got} != ${want}`
  );
}

// Bit 15 set: the sensor reports an error, not a temperature.
for (const raw of [0x8000, 0xffff, 0xbaf6]) {
  assert.strictEqual(
    decodeTemperature(raw),
    null,
    `decodeTemperature(0x${raw.toString(16)}) must be null (bit 15 set)`
  );
}

console.log(`OK: ${cases.length + 3} raw-word -> °C conversions match the Python node`);
