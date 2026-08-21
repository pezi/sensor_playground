#!/usr/bin/env node
'use strict';
/*
 * Differential test: temperature, pressure, humidity and heater values use
 * Pimoroni reference goldens; gas values correct its unsigned decoding of
 * Bosch's signed range-switch-error calibration field.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');
const fs = require('fs');
const path = require('path');

const { BME680Driver } = require('./bme680_driver');
const { BME680Sensor } = require('./sensor');

const goldens = JSON.parse(
  fs.readFileSync(path.join(__dirname, 'testdata', 'compensation_goldens.json'), 'utf8')
);

const driver = Object.create(BME680Driver.prototype);
driver._cal = {};
driver._tFine = 0n;
driver._ambient = 0n;
driver._setCalibration(
  Buffer.from(goldens.cal),
  goldens.heat_range_reg,
  goldens.heat_val,
  goldens.sw_err_reg
);
assert.strictEqual(driver._cal.rangeSwErr, -4n, 'signed range switch error');

let n = 0;
for (const c of goldens.cases) {
  const t = driver._calcTemperature(BigInt(c.adc_t));
  assert.strictEqual(Number(t), c.t, `case ${n}: temperature`);
  assert.strictEqual(Number(driver._tFine), c.tfine, `case ${n}: t_fine`);
  assert.strictEqual(Number(driver._calcPressure(BigInt(c.adc_p))), c.p, `case ${n}: pressure`);
  assert.strictEqual(Number(driver._calcHumidity(BigInt(c.adc_h))), c.h, `case ${n}: humidity`);
  const gas = driver._calcGasResistanceLow(BigInt(c.adc_g), c.range);
  assert.ok(Math.abs(gas - c.g) < 1e-6, `case ${n}: gas ${gas} != ${c.g}`);
  driver._ambient = t;
  assert.strictEqual(driver._calcHeaterResistance(320), c.heat, `case ${n}: heater`);
  n++;
}

async function testReadSerializationAndGasValidity() {
  const queuedDriver = new BME680Driver(null);
  let driverActive = 0;
  let maxDriverActive = 0;
  queuedDriver._readOnce = async () => {
    driverActive++;
    maxDriverActive = Math.max(maxDriverActive, driverActive);
    await new Promise((resolve) => setTimeout(resolve, 2));
    driverActive--;
    return null;
  };
  await Promise.all(Array.from({ length: 8 }, () => queuedDriver.read()));
  assert.strictEqual(maxDriverActive, 1, 'driver I2C transactions must be serialized');

  const sensor = new BME680Sensor();
  let active = 0;
  let maxActive = 0;
  sensor._driver = {
    async read() {
      active++;
      maxActive = Math.max(maxActive, active);
      await new Promise((resolve) => setTimeout(resolve, 2));
      active--;
      return {
        temperature: 22,
        humidity: 45,
        pressure: 1013,
        gasResistance: 120000,
        gasValid: true,
        heatStable: true,
      };
    },
  };
  await Promise.all(Array.from({ length: 8 }, () => sensor.read()));
  assert.strictEqual(maxActive, 1, 'sensor reads must be serialized');
  assert.strictEqual(sensor._iaq._next, 8, 'valid gas samples should advance IAQ');

  const invalidSensor = new BME680Sensor();
  invalidSensor._driver = {
    async read() {
      return {
        temperature: 22,
        humidity: 45,
        pressure: 1013,
        gasResistance: 0,
        gasValid: false,
        heatStable: true,
      };
    },
  };
  const invalid = await invalidSensor.read();
  assert.strictEqual(invalid.iaq, 0, 'invalid gas should retain the prior IAQ');
  assert.strictEqual(invalidSensor._iaq._next, 0, 'invalid gas must not advance IAQ');

  const staleDriver = new BME680Driver(null);
  staleDriver._cal = {};
  staleDriver._setCalibration(
    Buffer.from(goldens.cal),
    goldens.heat_range_reg,
    goldens.heat_val,
    goldens.sw_err_reg
  );
  staleDriver._setBits = async () => {};
  staleDriver._readReg = async (register) => register === 0x1e ? 7 : 0x80;
  let fieldReads = 0;
  staleDriver._readRegs = async () => {
    fieldReads++;
    const registers = Buffer.alloc(17);
    registers[1] = fieldReads === 1 ? 7 : 8;
    const sample = goldens.cases[0];
    registers[2] = sample.adc_p >> 12;
    registers[3] = sample.adc_p >> 4;
    registers[4] = (sample.adc_p & 0x0f) << 4;
    registers[5] = sample.adc_t >> 12;
    registers[6] = sample.adc_t >> 4;
    registers[7] = (sample.adc_t & 0x0f) << 4;
    registers[8] = sample.adc_h >> 8;
    registers[9] = sample.adc_h;
    registers[13] = sample.adc_g >> 2;
    registers[14] = ((sample.adc_g & 0x03) << 6) | sample.range | 0x30;
    return registers;
  };
  const fresh = await staleDriver.read();
  assert.strictEqual(fieldReads, 2, 'a stale measurement index must be ignored');
  assert.strictEqual(fresh.gasValid, true, 'gas-valid status bit should be decoded');
  assert.strictEqual(fresh.heatStable, true, 'heater-stable status bit should be decoded');
}

testReadSerializationAndGasValidity()
  .then(() => console.log(`OK: ${n} corrected golden cases and sensor-state checks passed`))
  .catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
