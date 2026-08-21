#!/usr/bin/env node
'use strict';
/*
 * Unit test: big-endian 16-bit decoding, the 16384 LSB-per-g conversion
 * (incl. negative values), the die-temperature formula and the
 * roll/pitch/g-force math, mirrored from the Python node's smbus2 access.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { s16be, decodeAccelTemp, axesToOrientation } = require('./mpu6050_driver');

const approx = (got, want, name) =>
  assert.ok(Math.abs(got - want) < 1e-9, `${name}: ${got} != ${want}`);

// Big-endian two's complement: 0x8000 wraps to -32768, 0xFFFF to -1.
assert.strictEqual(s16be(Buffer.from([0x00, 0x00]), 0), 0);
assert.strictEqual(s16be(Buffer.from([0x00, 0x01]), 0), 1);
assert.strictEqual(s16be(Buffer.from([0x7f, 0xff]), 0), 32767);
assert.strictEqual(s16be(Buffer.from([0x80, 0x00]), 0), -32768);
assert.strictEqual(s16be(Buffer.from([0xff, 0xff]), 0), -1);
assert.strictEqual(s16be(Buffer.from([0xc0, 0x00]), 0), -16384);

// Raw -> g with 16384 LSB per g, and raw/340 + 36.53 for the die
// temperature; the trailing gyroscope words are ignored.
const raw = Buffer.from([
  0x20, 0x00, // ax =   8192 -> 0.5 g
  0xc0, 0x00, // ay = -16384 -> -1.0 g
  0x40, 0x00, // az =  16384 -> 1.0 g
  0xf9, 0x5c, // temp = -1700 -> -5.0 + 36.53 = 31.53 °C
  0xaa, 0xaa, 0xaa, 0xaa, 0xaa, 0xaa, // gyroscope (unused)
]);
const d = decodeAccelTemp(raw);
approx(d.x, 0.5, 'x');
approx(d.y, -1.0, 'y');
approx(d.z, 1.0, 'z');
approx(d.temperature, 31.53, 'temperature');

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

console.log('OK: 16-bit decoding, g/temperature conversion and roll/pitch math match the Python node');
