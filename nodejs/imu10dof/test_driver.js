#!/usr/bin/env node
'use strict';
/*
 * Unit test: the MPU9250 accelerometer and AK8963 magnetometer scaling at
 * the configured full scales, the derived roll/pitch/heading/g-force math,
 * and a differential test of the BMP280 integer compensation against
 * golden values generated with the Python node's own BMP280 class.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');
const fs = require('fs');
const path = require('path');

const {
  convertAccel,
  convertMag,
  computeAngles,
  compensateBmp280,
} = require('./imu10dof_driver');

const approx = (got, want, name) =>
  assert.ok(Math.abs(got - want) < 1e-9, `${name}: ${got} != ${want}`);

// Big-endian raw counts at the 8 g full scale: 16384 -> 4 g, -8192 -> -2 g,
// 8192 -> 2 g.
const accel = convertAccel(Buffer.from([0x40, 0x00, 0xe0, 0x00, 0x20, 0x00]));
approx(accel.x, 4.0, 'accel x');
approx(accel.y, -2.0, 'accel y');
approx(accel.z, 2.0, 'accel z');

// Little-endian raw counts with the 4912/32760 µT-per-count scale:
// 3276 -> 491.2 µT, -3276 -> -491.2 µT, 32760 -> 4912 µT — each multiplied
// by its factory sensitivity coefficient.
const mag = convertMag(
  Buffer.from([0xcc, 0x0c, 0x34, 0xf3, 0xf8, 0x7f, 0x00]),
  [1.0, 1.0, 0.5]
);
approx(mag.x, 491.2, 'mag x');
approx(mag.y, -491.2, 'mag y');
approx(mag.z, 2456.0, 'mag z');

// A set ST2 overflow bit (0x08) discards the sample, like the
// mpu9250-jmdev reference driver.
const overflow = convertMag(
  Buffer.from([0xcc, 0x0c, 0x34, 0xf3, 0xf8, 0x7f, 0x08]),
  [1.0, 1.0, 1.0]
);
assert.deepStrictEqual(overflow, { x: 0, y: 0, z: 0 }, 'overflow yields zeros');

// A level board pointing magnetic north-east: no roll/pitch, 1 g,
// heading 45°.
let angles = computeAngles(0, 0, 1, 10, 10);
approx(angles.roll, 0, 'roll');
approx(angles.pitch, 0, 'pitch');
approx(angles.heading, 45.0, 'heading');
approx(angles.gforce, 1.0, 'gforce');

// Negative atan2 results normalize into [0, 360) like Python's %.
angles = computeAngles(0, 0, 1, 10, -10);
approx(angles.heading, 315.0, 'heading');

// 45° roll: y and z pull equally; nose-down: gravity on -x -> +90° pitch.
approx(computeAngles(0, 0.7, 0.7, 1, 0).roll, 45.0, 'roll');
approx(computeAngles(-1.0, 0, 0, 1, 0).pitch, 90.0, 'pitch');

// The BMP280 compensation against the Python goldens.
const goldens = JSON.parse(
  fs.readFileSync(path.join(__dirname, 'testdata', 'bmp280_goldens.json'), 'utf8')
);

let n = 0;
for (const c of goldens.cases) {
  const r = compensateBmp280(goldens.cal, c.adc_t, c.adc_p);
  assert.ok(Math.abs(r.temperature - c.t) < 1e-9, `case ${n}: temperature ${r.temperature} != ${c.t}`);
  assert.ok(Math.abs(r.pressure - c.p) < 1e-9, `case ${n}: pressure ${r.pressure} != ${c.p}`);
  n++;
}

console.log(
  `OK: accelerometer/magnetometer scaling and roll/pitch/heading math match the ` +
  `Python node, ${n} BMP280 golden cases match its compensation`
);
