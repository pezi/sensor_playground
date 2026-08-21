#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — TCS34725 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a TCS34725 I2C sensor (RGB colour,
 * colour temperature, illuminance).
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

const { loadConfig, hostnameOr } = require('../common/config');
const { HTTPS_PORT, startDiscoveryListener, runRestServer } = require('../common/wifi');

const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Convert a hue/saturation/value triple to red, green and blue in 0..1 —
 * the same sextant maths as Python's colorsys.hsv_to_rgb, used only to
 * drive the emulated colour cycle.
 */
function hsvToRGB(h, s, v) {
  if (s === 0.0) return [v, v, v];
  const sextant = Math.trunc(h * 6.0);
  const f = h * 6.0 - sextant;
  const p = v * (1.0 - s);
  const q = v * (1.0 - s * f);
  const t = v * (1.0 - s * (1.0 - f));
  switch (sextant % 6) {
    case 0: return [v, t, p];
    case 1: return [q, v, p];
    case 2: return [p, v, t];
    case 3: return [p, q, v];
    case 4: return [t, p, v];
    default: return [v, p, q];
  }
}

/*
 * Emulation: a coloured light slowly cycling through the hue circle every
 * 30 seconds, with the colour temperature swinging between warm and cool
 * white and the illuminance drifting around a few hundred lux (like the
 * Python node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  const [r, g, b] = hsvToRGB((t / 30.0) % 1.0, 0.6, 0.9);
  return {
    colorTemperature: Math.round(4600 + 1900 * Math.sin(t / 45.0)),
    lux: Math.round(300 + 200 * Math.sin(t / 75.0) + uniform(-5, 5)),
    red: Math.round(r * 255),
    green: Math.round(g * 255),
    blue: Math.round(b * 255),
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
    console.log('Emulation mode: generating TCS34725 readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing TCS34725 sensor on /dev/i2c-${i2cBus}...`);
    const { TCS34725Driver } = require('./tcs34725_driver');
    const driver = await TCS34725Driver.create(i2cBus);
    // A null reading means the clear channel saturated: there is no colour
    // to report, so the REST server answers 503.
    read = () => driver.read();
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return { ct: full.colorTemperature, lux: full.lux };
  };
  startDiscoveryListener('TCS34725', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('TCS34725', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
