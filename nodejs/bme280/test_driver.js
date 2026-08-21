#!/usr/bin/env node
'use strict';
/*
 * Differential test: the compensation formulas are checked against golden
 * values generated with the RPi.bme280 Python driver's double-precision
 * formulas (the library the Python node uses).
 *
 * Run: node test_driver.js
 */

const assert = require('assert');
const fs = require('fs');
const path = require('path');

const { compensate, parseCalibration } = require('./bme280_driver');

const goldens = JSON.parse(
  fs.readFileSync(path.join(__dirname, 'testdata', 'compensation_goldens.json'), 'utf8')
);

let n = 0;
for (const c of goldens.cases) {
  const r = compensate(goldens.cal, c.raw_t, c.raw_p, c.raw_h);
  assert.ok(Math.abs(r.temperature - c.t) < 1e-9, `case ${n}: temperature ${r.temperature} != ${c.t}`);
  assert.ok(Math.abs(r.pressure - c.p) < 1e-9, `case ${n}: pressure ${r.pressure} != ${c.p}`);
  assert.ok(Math.abs(r.humidity - c.h) < 1e-9, `case ${n}: humidity ${r.humidity} != ${c.h}`);
  n++;
}

// H4/H5 shared-nibble assembly from signed byte reads (as RPi.bme280 does).
const cal = parseCalibration(Buffer.alloc(24), 75, Buffer.from([0x78, 0x01, 0x00, 0x11, 0xc8, 0x1e, 0x1e]));
assert.strictEqual(cal.H4, 280, `H4 ${cal.H4} != 280`);
assert.strictEqual(cal.H5, 492, `H5 ${cal.H5} != 492`);

console.log(`OK: ${n} golden cases match the RPi.bme280 reference driver`);
