#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — BMP085 barometer (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a BMP085 I2C barometer — the sensor
 * behind the Grove Barometer Sensor. Reports temperature, barometric
 * pressure, and the altitude derived from it. The pin-compatible BMP180
 * works unchanged.
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
const { altitudeFor } = require('./bmp085_driver');

const round1 = (x) => Math.round(x * 10) / 10;
const round2 = (x) => Math.round(x * 100) / 100;
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a room near sea level, drifting on slow sines; the altitude
 * is derived from the emulated pressure so the values stay consistent.
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  const pressureHpa = 1013.0 + 3.0 * Math.sin(t / 300.0) + uniform(-0.2, 0.2);
  return {
    temperature: round1(21.0 + 2.0 * Math.sin(t / 60.0) + uniform(-0.1, 0.1)),
    pressure: round2(pressureHpa),
    altitude: round1(altitudeFor(pressureHpa * 100.0)),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;
  const oversampling = config.oversampling ?? 3;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating BMP085 readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing BMP085 sensor on /dev/i2c-${i2cBus}...`);
    const { BMP085Driver } = require('./bmp085_driver');
    const driver = await BMP085Driver.create(i2cBus, oversampling);
    read = async () => {
      const r = await driver.read();
      return {
        temperature: round1(r.temperature),
        pressure: round2(r.pressurePa / 100.0), // Pa -> hPa
        altitude: round1(altitudeFor(r.pressurePa)),
      };
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return { temp: full.temperature, press: full.pressure, alt: full.altitude };
  };
  startDiscoveryListener('BMP085', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('BMP085', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
