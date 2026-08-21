#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — BME680 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a BME680 I2C sensor (temperature,
 * humidity, pressure, IAQ).
 *
 * - HTTPS REST API on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   Wi-Fi with a warning (use the Python node for BLE).
 *
 * Set "emulation": true in config.json to generate plausible readings
 * without the sensor hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

// The shared transport helpers live in the sibling folder (deploy
// ../common next to this node folder, like the Python nodes).
const { loadConfig, hostnameOr } = require('../common/config');
const { HTTPS_PORT, startDiscoveryListener, runRestServer } = require('../common/wifi');
const { BME680Sensor, EmulatedBME680Sensor } = require('./sensor');

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let sensor;
  if (config.emulation) {
    console.log('Emulation mode: generating BME680 readings without hardware');
    sensor = new EmulatedBME680Sensor();
  } else {
    console.log(`Initializing BME680 sensor on /dev/i2c-${i2cBus}...`);
    sensor = await BME680Sensor.create(i2cBus);
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await sensor.read();
    if (full === null) return {};
    return { temp: full.temperature, hum: full.humidity, press: full.pressure, iaq: full.iaq };
  };
  startDiscoveryListener(sensor.name, hostname, HTTPS_PORT, readDiscovery);
  runRestServer(sensor.name, config.api_key, hostname, () => sensor.read(), sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
