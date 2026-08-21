#!/usr/bin/env node
'use strict';
/*
 * The CRC-8 is checked against the SHT3x datasheet example
 * (CRC(0xBEEF) = 0x92) and the conversion formulas against exact raw
 * values from the datasheet formulas (§4.13).
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { crc8, convertTemperature, convertHumidity, parseFrame } = require('./sht31_driver');

// Datasheet CRC example.
assert.strictEqual(crc8(Buffer.from([0xbe, 0xef])), 0x92, 'CRC(0xBEEF) != 0x92');

// Conversion formulas at exact points.
const near = (a, b) => Math.abs(a - b) < 1e-9;
assert.ok(near(convertTemperature(0x0000), -45.0), 'temperature(0)');
assert.ok(near(convertTemperature(0xffff), 130.0), 'temperature(65535)');
assert.ok(near(convertTemperature(26214), 25.0), 'temperature(26214)'); // 26214/65535 = 2/5
assert.ok(near(convertHumidity(0x0000), 0.0), 'humidity(0)');
assert.ok(near(convertHumidity(0xffff), 100.0), 'humidity(65535)');
assert.ok(near(convertHumidity(26214), 40.0), 'humidity(26214)');

// Frame parsing: temp raw 26214 (25.0 °C), hum raw 26214 (40.0 %RH).
const word = Buffer.from([0x66, 0x66]);
const frame = Buffer.from([0x66, 0x66, crc8(word), 0x66, 0x66, crc8(word)]);
const r = parseFrame(frame);
assert.ok(near(r.temperature, 25.0), `temperature ${r.temperature} != 25.0`);
assert.ok(near(r.humidity, 40.0), `humidity ${r.humidity} != 40.0`);

// Corrupted CRC and corrupted data word must be detected.
const badCrc = Buffer.from(frame);
badCrc[2] ^= 0xff;
assert.throws(() => parseFrame(badCrc), /temperature CRC/);
const badWord = Buffer.from(frame);
badWord[4] ^= 0x01;
assert.throws(() => parseFrame(badWord), /humidity CRC/);

console.log('OK: CRC-8 matches the datasheet example, conversions and frame parsing check out');
