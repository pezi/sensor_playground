#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — SCD30 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a SCD30 I2C sensor (temperature,
 * humidity, CO2).
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
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a quiet indoor room — temperature and humidity follow slow
 * sines with different periods, the CO2 concentration does a bounded
 * random walk between 400 and 1500 ppm.
 */
function makeEmulatedRead() {
  let co2 = 600.0;
  return () => {
    const t = Date.now() / 1000;
    co2 = Math.min(Math.max(co2 + uniform(-15, 15), 400.0), 1500.0);
    return {
      temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0) + uniform(-0.1, 0.1)),
      humidity: round1(45.0 + 8.0 * Math.sin(t / 97.0) + uniform(-0.5, 0.5)),
      co2: round1(co2),
    };
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
    console.log('Emulation mode: generating SCD30 readings without hardware');
    const emulatedRead = makeEmulatedRead();
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing SCD30 sensor on /dev/i2c-${i2cBus}...`);
    const { SCD30Driver } = require('./scd30_driver');
    const driver = await SCD30Driver.create(i2cBus);
    read = async () => {
      // null while no fresh measurement is ready (2 s interval) -> 503,
      // like the Python node's read() returning None.
      const m = await driver.read();
      if (!m) return null;
      return {
        temperature: round1(m.temperature),
        humidity: round1(m.humidity),
        co2: round1(m.co2),
      };
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return { temp: full.temperature, hum: full.humidity, co2: full.co2 };
  };
  startDiscoveryListener('SCD30', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('SCD30', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
