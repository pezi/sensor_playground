#!/usr/bin/env node
'use strict';

const assert = require('assert');
const { emulatedRead, toDiscovery } = require('./sensor_node');

const reading = emulatedRead(60, 0);
const expectedTemperature = Math.round((22.0 + 2.0 * Math.sin(1)) * 10) / 10;
const expectedHumidity = Math.round((45.0 + 8.0 * Math.sin(60 / 97)) * 10) / 10;

assert.deepStrictEqual(reading, {
  temperature: expectedTemperature,
  humidity: expectedHumidity,
});
assert.deepStrictEqual(toDiscovery(reading), {
  temp: expectedTemperature,
  hum: expectedHumidity,
});
assert.deepStrictEqual(toDiscovery(null), {});

for (const value of Object.values(reading)) {
  assert.ok(Number.isInteger(value * 10), `${value} is not at one-decimal precision`);
}

console.log('OK: DHT22 emulation precision and discovery payload');
