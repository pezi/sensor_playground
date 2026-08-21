#!/usr/bin/env node
'use strict';
/*
 * Unit test: the colour maths must match the Python node's — the library's
 * gamma-corrected RGB bytes and the DN40 lux / colour temperature
 * algorithm. The expected values below were produced by running the
 * reference Python implementation on the same raw counts.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { colorRGBBytes, temperatureAndLuxDN40 } = require('./tcs34725_driver');

// The node's profile: 154 ms requested -> 64 cycles -> 153.6 ms, at 4x gain.
const INTEGRATION_MS = 153.6;
const GAIN = 4;

const rgbCases = [
  [1000, 1200, 900, 3500, 11, 17, 8],
  [5000, 4200, 3000, 12000, 28, 18, 8],
  [300, 400, 350, 900, 16, 33, 23],
  // Clear at zero is complete darkness: black, not a divide by zero.
  [0, 0, 0, 0, 0, 0, 0],
];
for (const [r, g, b, c, wantR, wantG, wantB] of rgbCases) {
  const got = colorRGBBytes(r, g, b, c);
  assert.deepStrictEqual(
    got,
    { red: wantR, green: wantG, blue: wantB },
    `colorRGBBytes(${r},${g},${b},${c}) = ${JSON.stringify(got)}`
  );
}

// A fully lit channel saturates at 255 rather than overflowing past it.
assert.deepStrictEqual(colorRGBBytes(65535, 65535, 65535, 65535), {
  red: 255,
  green: 255,
  blue: 255,
});

const dn40Cases = [
  [1000, 1200, 900, 3500, 472.46744791666663, 4820.0],
  [5000, 4200, 3000, 12000, 1755.2539062499998, 3645.8979591836733],
  [300, 400, 350, 900, 117.81412760416667, 6047.666666666667],
  // All channels dark: no light and the bare CT offset.
  [0, 0, 0, 0, 0.0, 1391.0],
];
for (const [r, g, b, c, wantLux, wantCT] of dn40Cases) {
  const got = temperatureAndLuxDN40(r, g, b, c, INTEGRATION_MS, GAIN);
  assert.ok(got !== null, `temperatureAndLuxDN40(${r},${g},${b},${c}) reported saturation`);
  assert.ok(Math.abs(got.lux - wantLux) < 1e-9, `lux = ${got.lux}, want ${wantLux}`);
  assert.ok(
    Math.abs(got.colorTemperature - wantCT) < 1e-9,
    `colorTemperature = ${got.colorTemperature}, want ${wantCT}`
  );
}

// A saturated clear channel says nothing about the colour, so the sample is
// rejected instead of being reported as a very bright reading.
assert.strictEqual(temperatureAndLuxDN40(60000, 60000, 60000, 65535, INTEGRATION_MS, GAIN), null);
// Below 150 ms the saturation limit drops by a quarter (DN40 3.7): at
// 100.8 ms the limit is 1024*42*0.75 = 32256 counts.
assert.strictEqual(temperatureAndLuxDN40(9000, 9000, 9000, 32256, 100.8, GAIN), null);
const shortIntegration = temperatureAndLuxDN40(1000, 1200, 900, 3500, 100.8, GAIN);
assert.ok(
  Math.abs(shortIntegration.lux - 719.9503968253969) < 1e-9,
  `lux at 100.8 ms = ${shortIntegration.lux}`
);

console.log(`OK: ${rgbCases.length + dn40Cases.length} colour conversions match the Python node`);
