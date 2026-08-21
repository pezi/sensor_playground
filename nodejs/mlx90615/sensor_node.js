#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — MLX90615 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a Grove Digital Infrared Temperature
 * Sensor (MLX90615): the non-contact object temperature plus the sensor's
 * own ambient temperature.
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
 * Emulation: an ambient temperature drifting on a slow sine around 22 °C
 * and a warmer object (around 28 °C) in the field of view following its
 * own slower sine, both with a little measurement noise (like the Python
 * node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  return {
    temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0) + uniform(-0.1, 0.1)),
    objectTemperature: round1(28.0 + 3.0 * Math.sin(t / 45.0) + uniform(-0.2, 0.2)),
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
    console.log('Emulation mode: generating MLX90615 readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing MLX90615 sensor on /dev/i2c-${i2cBus}...`);
    const { MLX90615Driver } = require('./mlx90615_driver');
    const driver = await MLX90615Driver.create(i2cBus);
    read = async () => {
      const reading = await driver.read();
      // The sensor flagged an error on at least one channel; report no
      // reading rather than a bogus temperature.
      if (reading === null) return null;
      return {
        temperature: round1(reading.ambient),
        objectTemperature: round1(reading.object),
      };
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return { temp: full.temperature, objtemp: full.objectTemperature };
  };
  startDiscoveryListener('MLX90615', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('MLX90615', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
