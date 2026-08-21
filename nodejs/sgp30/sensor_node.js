#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — SGP30 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a SGP30 I2C sensor (eCO2, TVOC).
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

const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: indoor air quality drifting around typical office values —
 * eCO2 does a bounded random walk between 400 and 1500 ppm, TVOC between
 * 0 and 600 ppb (like the Python node).
 */
function emulatedReader() {
  let eco2 = 600.0;
  let tvoc = 60.0;
  return async () => {
    eco2 = Math.min(Math.max(eco2 + uniform(-15, 15), 400.0), 1500.0);
    tvoc = Math.min(Math.max(tvoc + uniform(-8, 8), 0.0), 600.0);
    return { eco2: Math.round(eco2), tvoc: Math.round(tvoc) };
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
    console.log('Emulation mode: generating SGP30 readings without hardware');
    read = emulatedReader();
  } else {
    console.log(`Initializing SGP30 sensor on /dev/i2c-${i2cBus}...`);
    const { SGP30Driver } = require('./sgp30_driver');
    const driver = await SGP30Driver.create(i2cBus);
    console.log('Warming up SGP30 (about 15 seconds)...');
    await driver.startMeasurement();
    read = async () => driver.read();
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  // The Python node's read_discovery returns the same full keys as read.
  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return full;
  };
  startDiscoveryListener('SGP30', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('SGP30', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
