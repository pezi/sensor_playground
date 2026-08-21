#!/usr/bin/env node
'use strict';
/*
 * The raw-to-value conversion is checked against golden values computed
 * with the Python node's formulas (20-bit humidity / 20-bit temperature
 * split across the shared nibble in byte 3). The AHT10/AHT20 protocol the
 * Python node uses reads 6 bytes and has no CRC.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { AHT20Driver, parseReading } = require('./aht20_driver');

const cases = [
  // All-zero raw values -> scale minimum.
  { raw: [0x1c, 0x00, 0x00, 0x00, 0x00, 0x00], t: -50.0, h: 0.0 },
  // Mid-scale on both channels (0x80000 of 0x100000).
  { raw: [0x1c, 0x80, 0x00, 0x08, 0x00, 0x00], t: 50.0, h: 50.0 },
  // All-ones raw values -> scale maximum.
  { raw: [0x1c, 0xff, 0xff, 0xff, 0xff, 0xff], t: 149.99980926513672, h: 99.99990463256836 },
  // Realistic indoor reading, shared nibble in byte 3 non-zero.
  { raw: [0x1c, 0x6e, 0x14, 0x85, 0xc7, 0x2a], t: 22.224807739257812, h: 43.000030517578125 },
  // AHT10-style status byte (calibrated bit only).
  { raw: [0x08, 0x5d, 0x2e, 0x95, 0x8c, 0x51], t: 19.35138702392578, h: 36.399173736572266 },
];

let n = 0;
for (const c of cases) {
  const r = parseReading(Buffer.from(c.raw));
  assert.ok(r, `case ${n}: unexpected busy`);
  assert.ok(Math.abs(r.temperature - c.t) < 1e-12, `case ${n}: temperature ${r.temperature} != ${c.t}`);
  assert.ok(Math.abs(r.humidity - c.h) < 1e-12, `case ${n}: humidity ${r.humidity} != ${c.h}`);
  n++;
}

// A set busy bit (0x80 in the status byte) must yield no reading, like
// the Python node returning None.
assert.strictEqual(parseReading(Buffer.from([0x80, 0x12, 0x34, 0x56, 0x78, 0x9a])), null);

(async () => {
  const events = [];
  let transactionActive = false;
  let overlap = false;
  const bus = {
    async i2cWrite(_address, length) {
      if (transactionActive) overlap = true;
      transactionActive = true;
      events.push('write');
      return { bytesWritten: length };
    },
    async i2cRead(_address, length, buffer) {
      Buffer.from([0x1c, 0x80, 0x00, 0x08, 0x00, 0x00]).copy(buffer);
      transactionActive = false;
      events.push('read');
      return { bytesRead: length, buffer };
    },
  };

  const driver = new AHT20Driver(bus);
  await Promise.all([driver.read(), driver.read()]);
  assert.strictEqual(overlap, false, 'trigger/wait/read transactions must not overlap');
  assert.deepStrictEqual(events, ['write', 'read', 'write', 'read']);

  console.log(`OK: ${n} golden cases and serialized sensor reads`);
})().catch((err) => {
  console.error(err);
  process.exitCode = 1;
});
