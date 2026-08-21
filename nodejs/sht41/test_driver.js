#!/usr/bin/env node
'use strict';
/*
 * Unit tests for the portable SHT41 driver math: the Sensirion CRC-8
 * (datasheet example 0xBE 0xEF -> 0x92), frame parsing and the datasheet
 * conversion formulas (0x6666 = exactly 40% full scale, so temperature
 * 25.0 °C and humidity 44.0 %RH fall out exactly).
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { crc8, parseFrame, convert } = require('./sht41_driver');

// CRC-8 known vectors.
assert.strictEqual(crc8([0xbe, 0xef]), 0x92); // SHT4x datasheet example
assert.strictEqual(crc8([0x00, 0x00]), 0x81);
assert.strictEqual(crc8([0x66, 0x66]), 0x93);
assert.strictEqual(crc8([0x80, 0x00]), 0xa2);
assert.strictEqual(crc8([0xff, 0xff]), 0xac);

// Frame parsing: valid frame, corrupted CRCs, short frame.
const { tempRaw, humRaw } = parseFrame(Buffer.from([0x66, 0x66, 0x93, 0x80, 0x00, 0xa2]));
assert.strictEqual(tempRaw, 0x6666);
assert.strictEqual(humRaw, 0x8000);
assert.throws(() => parseFrame(Buffer.from([0x66, 0x66, 0x94, 0x80, 0x00, 0xa2])), /CRC/);
assert.throws(() => parseFrame(Buffer.from([0x66, 0x66, 0x93, 0x80, 0x00, 0xa3])), /CRC/);
assert.throws(() => parseFrame(Buffer.from([0x66, 0x66, 0x93])), /6/);

// Conversions, incl. the humidity clamp to 0..100.
const near = (a, b) => Math.abs(a - b) < 1e-9;
let r = convert(0x0000, 0x0000);
assert.ok(near(r.temperature, -45.0) && near(r.humidity, 0.0), 'raw 0x0000');
r = convert(0xffff, 0xffff);
assert.ok(near(r.temperature, 130.0) && near(r.humidity, 100.0), 'raw 0xffff');
r = convert(0x6666, 0x6666);
assert.ok(near(r.temperature, 25.0) && near(r.humidity, 44.0), 'raw 0x6666');
r = convert(0x8000, 0x8000);
assert.ok(near(r.temperature, 42.50133516441596), 'raw 0x8000 temperature');
assert.ok(near(r.humidity, 56.50095368886854), 'raw 0x8000 humidity');

console.log('OK: CRC-8 vectors, frame parsing and conversions match the reference');
