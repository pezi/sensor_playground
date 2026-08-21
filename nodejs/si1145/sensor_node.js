#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — SI1145 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a SI1145 I2C sensor (visible light,
 * infrared, UV index).
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
 * Emulation: indoor light near a window — visible and IR counts hover
 * around the chip's dark baseline (~260) with slow sines and a little
 * flicker, while the UV index sweeps between 0 and 8 over ten minutes
 * (like the Python node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  return {
    visible: Math.round(262 + 40 * Math.sin(t / 90.0) + uniform(-3, 3)),
    ir: Math.round(253 + 30 * Math.sin(t / 70.0) + uniform(-3, 3)),
    uv: round1(4.0 + 4.0 * Math.sin(t / 600.0)),
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
    console.log('Emulation mode: generating SI1145 readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing SI1145 sensor on /dev/i2c-${i2cBus}...`);
    const { SI1145Driver } = require('./si1145_driver');
    const driver = await SI1145Driver.create(i2cBus);
    read = async () => {
      const { visible, ir, uv } = await driver.read();
      return { visible, ir, uv: round1(uv) };
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return { vis: full.visible, ir: full.ir, uv: full.uv };
  };
  startDiscoveryListener('SI1145', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('SI1145', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
