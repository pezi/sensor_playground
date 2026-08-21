#!/usr/bin/env node
'use strict';
/*
 * Driver unit test: the worked example from the BMP085 datasheet
 * (section 3.5) plus calibration-garbage rejection.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { compensate, parseCalibration, altitudeFor, SEA_LEVEL_PA } = require('./bmp085_driver');

// Datasheet worked example: these constants with raw_t=27898, raw_p=23843
// at oversampling 0 compensate to 15.0 °C and 69964 Pa.
const cal = {
  ac1: 408, ac2: -72, ac3: -14383, ac4: 32741, ac5: 32757, ac6: 23153,
  b1: 6190, b2: 4, mb: -32768, mc: -8711, md: 2868,
};
const { temperature, pressurePa } = compensate(cal, 27898, 23843, 0);
assert.strictEqual(temperature, 15.0, `temperature ${temperature} != 15.0`);
assert.strictEqual(pressurePa, 69964, `pressure ${pressurePa} != 69964`);

assert.ok(Math.abs(altitudeFor(SEA_LEVEL_PA)) < 1e-9, 'altitude at sea level != 0');
assert.ok(Math.abs(altitudeFor(69964) - 3021) < 30, 'altitude for 69964 Pa not ~3021 m');

assert.throws(() => parseCalibration(Buffer.alloc(22)), /implausible/);
assert.throws(() => parseCalibration(Buffer.alloc(22, 0xff)), /implausible/);

console.log('OK: BMP085 compensation matches the datasheet worked example');
