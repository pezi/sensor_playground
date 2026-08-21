#!/usr/bin/env node
'use strict';
/*
 * The platform-neutral driver math — timeout register coding, MCLK/µs
 * conversion, sequence step decoding, timing-budget arithmetic and
 * reference SPAD map masking — checked against golden values generated
 * with the adafruit_vl53l0x reference driver.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const {
  decodeTimeout,
  encodeTimeout,
  timeoutMclksToUs,
  timeoutUsToMclks,
  decodeVcselPeriod,
  sequenceStepEnables,
  maskRefSpadMap,
} = require('./vl53l0x_driver');

// Timeout register decoding/encoding ("(LSByte * 2^MSByte) + 1"); 0x0096
// and 0x01FE are the pre/final range timeouts the tuning settings write.
for (const [reg, mclks] of [[0x0096, 151], [0x01fe, 509], [0x0180, 257]]) {
  assert.strictEqual(decodeTimeout(reg), mclks, `decode(0x${reg.toString(16)})`);
  assert.strictEqual(encodeTimeout(mclks), reg, `encode(${mclks})`);
}
assert.strictEqual(encodeTimeout(0), 0);

// VCSEL periods of the tuning defaults (pre-range register 0x06 -> 14
// PCLKs, final-range 0x04 -> 10 PCLKs) and the MCLK/µs conversions.
assert.strictEqual(decodeVcselPeriod(0x06), 14);
assert.strictEqual(decodeVcselPeriod(0x04), 10);
assert.strictEqual(timeoutMclksToUs(38, 14), 2055); // MSRC (register 0x25 -> 38 MCLKs)
assert.strictEqual(timeoutMclksToUs(151, 14), 8087); // pre range
assert.strictEqual(timeoutMclksToUs(509 - 151, 10), 13669); // final minus pre range
assert.strictEqual(timeoutUsToMclks(14259, 10), 374);

// 0xE8 is the sequence config the driver leaves active.
assert.deepStrictEqual(sequenceStepEnables(0xe8), {
  tcc: false, dss: true, msrc: false, preRange: true, finalRange: true,
});
assert.deepStrictEqual(sequenceStepEnables(0xff), {
  tcc: true, dss: true, msrc: true, preRange: true, finalRange: true,
});

// The timing budget the getter computes from the tuning-default registers,
// and the final range timeout the setter re-encodes for the same budget.
const msrcDssTccUs = timeoutMclksToUs(38, 14);
const preRangeUs = timeoutMclksToUs(151, 14);
const finalRangeUs = timeoutMclksToUs(509 - 151, 10);
const budgetUs = 1910 + 960 + 2 * (msrcDssTccUs + 690) + (preRangeUs + 660) + (finalRangeUs + 550);
assert.strictEqual(budgetUs, 31326, `timing budget ${budgetUs}`);
const usedBudgetUs = 1320 + 960 + 2 * (msrcDssTccUs + 690) + (preRangeUs + 660) + 550;
const finalRangeTimeoutMclks = timeoutUsToMclks(budgetUs - usedBudgetUs, 10) + 151;
assert.strictEqual(encodeTimeout(finalRangeTimeoutMclks), 0x0283);

// Reference SPAD map masking.
const cases = [
  // Aperture SPADs start at bit 12; the first 12 bits are cleared.
  [[0xff, 0xff, 0xff, 0xff, 0xff, 0xff], 5, true, [0x00, 0xf0, 0x01, 0x00, 0x00, 0x00], 5],
  [[0xff, 0xff, 0xff, 0xff, 0xff, 0xff], 3, false, [0x07, 0x00, 0x00, 0x00, 0x00, 0x00], 3],
  // Holes in the good SPAD map are skipped, not counted.
  [[0x00, 0xcc, 0x00, 0x00, 0x00, 0x00], 2, true, [0x00, 0xc0, 0x00, 0x00, 0x00, 0x00], 2],
  // Asking for more SPADs than exist enables all remaining ones.
  [[0xff, 0xff, 0xff, 0xff, 0xff, 0xff], 48, true, [0x00, 0xf0, 0xff, 0xff, 0xff, 0xff], 36],
];
for (const [input, count, isAperture, want, wantEnabled] of cases) {
  const map = Buffer.from(input);
  const enabled = maskRefSpadMap(map, count, isAperture);
  assert.deepStrictEqual([...map], want, `map for count ${count}`);
  assert.strictEqual(enabled, wantEnabled, `enabled for count ${count}`);
}

console.log('OK: driver math matches the adafruit_vl53l0x reference driver');
