#!/usr/bin/env node
'use strict';
/*
 * Unit test: 6-bit two's-complement axis decoding, the 21.33 counts-per-g
 * conversion and the roll/pitch/g-force math, mirrored from the Python
 * node's smbus2 access.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { decodeAxis, decodeAxes, axesToOrientation } = require('./mma7660_driver');

const approx = (got, want, name) =>
  assert.ok(Math.abs(got - want) < 1e-9, `${name}: ${got} != ${want}`);

// 6-bit two's complement: 0..31 positive, 32..63 wrap to -32..-1.
assert.strictEqual(decodeAxis(0), 0);
assert.strictEqual(decodeAxis(1), 1);
assert.strictEqual(decodeAxis(31), 31);
assert.strictEqual(decodeAxis(32), -32);
assert.strictEqual(decodeAxis(63), -1);
assert.strictEqual(decodeAxis(43), -21);

// Counts -> g with 21.33 counts per g.
const axes = decodeAxes(Buffer.from([21, 63, 32]));
approx(axes[0], 21 / 21.33, 'x');
approx(axes[1], -1 / 21.33, 'y');
approx(axes[2], -32 / 21.33, 'z');

// The alert bit (0x40) invalidates the whole block.
assert.strictEqual(decodeAxes(Buffer.from([0x40, 0, 21])), null);
assert.strictEqual(decodeAxes(Buffer.from([0, 0x55, 21])), null);

// Flat board resting at 1 g: no roll, no pitch.
let o = axesToOrientation(0, 0, 1.0);
approx(o.roll, 0, 'roll');
approx(o.pitch, 0, 'pitch');
approx(o.gforce, 1.0, 'gforce');

// 45° roll: y and z pull equally.
o = axesToOrientation(0, 0.7, 0.7);
approx(o.roll, 45, 'roll');
approx(o.pitch, 0, 'pitch');

// Nose-down: gravity entirely on -x -> +90° pitch.
o = axesToOrientation(-1.0, 0, 0);
approx(o.pitch, 90, 'pitch');
approx(o.gforce, 1.0, 'gforce');

console.log('OK: axis decoding, g conversion and roll/pitch math match the Python node');
