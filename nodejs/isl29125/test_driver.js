#!/usr/bin/env node
'use strict';
/*
 * Unit test: the raw counts → payload derivation must match the Python
 * node — lux from the green channel, the colour normalized against the
 * brightest channel, and no colour at all in complete darkness.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { deriveReading } = require('./isl29125_driver');

// Red is the brightest channel, so it saturates at 255 and the others are
// scaled against it; green drives the illuminance (30000 * 10000/65535).
const lit = deriveReading(30000, 60000, 15000);
assert.strictEqual(lit.lux, 4578, `lux = ${lit.lux}`);
assert.strictEqual(lit.red, 255, `red = ${lit.red}`);
assert.strictEqual(lit.green, 128, `green = ${lit.green}`);
assert.strictEqual(lit.blue, 64, `blue = ${lit.blue}`);

// Full scale: the top of the 10K lux range, white.
const white = deriveReading(65535, 65535, 65535);
assert.strictEqual(white.lux, 10000, `lux = ${white.lux}`);
for (const key of ['red', 'green', 'blue']) {
  assert.strictEqual(white[key], 255, `${key} = ${white[key]}`);
}

// Complete darkness: an illuminance of 0 and no colour keys at all.
const dark = deriveReading(0, 0, 0);
assert.strictEqual(dark.lux, 0, `lux = ${dark.lux}`);
for (const key of ['red', 'green', 'blue']) {
  assert.ok(!(key in dark), `${key} must be absent in complete darkness`);
}

console.log('OK: 3 counts -> payload derivations match the Python node');
