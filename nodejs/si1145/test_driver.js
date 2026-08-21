#!/usr/bin/env node
'use strict';
/*
 * Unit test: the chip reports the UV index multiplied by 100, and the REST
 * payload rounds it to one decimal — both must match the Python node.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { uvIndex } = require('./si1145_driver');

const round1 = (x) => Math.round(x * 10) / 10;

const cases = [
  [0, 0.0],
  [50, 0.5],
  [123, 1.23],   // the payload rounds this to 1.2
  [800, 8.0],    // "very high" on the UV index scale
  [65535, 655.35], // full scale; far beyond any real UV index
];

for (const [raw, want] of cases) {
  const got = uvIndex(raw);
  assert.ok(Math.abs(got - want) < 1e-9, `uvIndex(${raw}) = ${got} != ${want}`);
}

// The REST payload reports one decimal, like the Python node.
assert.strictEqual(round1(uvIndex(123)), 1.2);
assert.strictEqual(round1(uvIndex(475)), 4.8);

console.log(`OK: ${cases.length} UV index conversions match the Python node`);
