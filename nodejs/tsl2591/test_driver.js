#!/usr/bin/env node
'use strict';
/*
 * Unit test: the lux equation must match the Python node's (the Adafruit
 * library's two-approximation formula), including the
 * integration-time-dependent saturation limit. The expected values below
 * were produced by running the reference Python implementation on the same
 * counts.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { calculateLux, GAIN_NAMES } = require('./tsl2591_driver');

const cases = [
  // channel0, channel1, integration register, gain factor, want
  [5400, 1500, 2, 25.0, 159.936],              // 300 ms, med gain — the defaults
  [200, 60, 0, 1.0, 414.5280000000001],        // 100 ms, low gain — bright sun
  [37000, 12000, 1, 428.0, 82.55327102803739], // 200 ms, high gain
  [100, 0, 5, 9876.0, 0.00688537869582827],    // 600 ms, max gain — near darkness
];
for (const [c0, c1, integration, gain, want] of cases) {
  const got = calculateLux(c0, c1, integration, gain);
  assert.ok(got !== null, `calculateLux(${c0},${c1},${integration},${gain}) reported saturation`);
  assert.ok(Math.abs(got - want) < 1e-9, `calculateLux(${c0},${c1}) = ${got}, want ${want}`);
}

// At the shortest integration time the ADC only counts to 0x8FFF, so the
// same count means saturation at 100 ms but not at 300 ms.
assert.strictEqual(calculateLux(0x8fff, 100, 0, 1.0), null);
const at300ms = calculateLux(0x8fff, 100, 2, 25.0);
assert.ok(Math.abs(at300ms - 1996.4256) < 1e-9, `0x8FFF at 300 ms = ${at300ms}`);
// A saturated infrared channel must be rejected too.
assert.strictEqual(calculateLux(1000, 0xffff, 2, 25.0), null);

assert.deepStrictEqual(GAIN_NAMES, ['low', 'med', 'high', 'max']);

console.log(`OK: ${cases.length} lux conversions match the Python node`);
