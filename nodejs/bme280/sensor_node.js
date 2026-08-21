#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — BME280 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a BME280 I2C sensor (temperature,
 * humidity, pressure).
 *
 * - HTTPS REST API on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   Wi-Fi with a warning (use the Python or Rust node for BLE).
 *
 * Set "emulation": true in config.json to generate plausible readings
 * without the sensor hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { HTTPS_PORT, startDiscoveryListener, runRestServer } = require('../common/wifi');

const round1 = (x) => Math.round(x * 10) / 10;
const round2 = (x) => Math.round(x * 100) / 100;
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a comfortable indoor climate drifting on slow sines around
 * 22 °C / 45 %RH / 1013 hPa, plus a little measurement noise.
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  return {
    temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0) + uniform(-0.1, 0.1)),
    humidity: round1(45.0 + 8.0 * Math.sin(t / 97.0) + uniform(-0.5, 0.5)),
    pressure: round2(1013.0 + 3.0 * Math.sin(t / 300.0) + uniform(-0.2, 0.2)),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating BME280 readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing BME280 sensor on /dev/i2c-${i2cBus}...`);
    const { BME280Driver } = require('./bme280_driver');
    const driver = await BME280Driver.create(i2cBus);
    read = async () => {
      const r = await driver.read();
      return {
        temperature: round1(r.temperature),
        humidity: round1(r.humidity),
        pressure: round2(r.pressure),
      };
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return { temp: full.temperature, hum: full.humidity, press: full.pressure };
  };
  startDiscoveryListener('BME280', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('BME280', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
