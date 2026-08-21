#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — SparkFun ISL29125 RGB Light Sensor (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers with a SparkFun RGB Light Sensor breakout — an
 * Intersil/Renesas ISL29125 (I2C address 0x44) measuring the intensity of
 * red, green and blue light while rejecting infrared. The channels are
 * normalized against the brightest one so the app can show the measured
 * colour directly (JSON keys red/green/blue, 0-255, absent in complete
 * darkness); an approximate illuminance (lux) is derived from the green
 * channel. There is no clear channel, so unlike the TCS34725 no colour
 * temperature is reported.
 * https://www.sparkfun.com/sparkfun-rgb-light-sensor-isl29125.html
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
const { deriveReading, LUX_PER_COUNT } = require('./isl29125_driver');

/*
 * Emulation: indoor light slowly shifting between warm and cool white,
 * with the brightness breathing over a couple of minutes (like the Python
 * node).
 */
function emulatedChannels() {
  const t = Date.now() / 1000;
  const brightness = 0.35 + 0.3 * Math.sin(t / 120.0);
  const warmth = 0.5 + 0.5 * Math.sin(t / 45.0);
  return {
    green: Math.round(65535 * brightness),
    red: Math.round(65535 * brightness * (0.6 + 0.4 * warmth)),
    blue: Math.round(65535 * brightness * (1.0 - 0.5 * warmth)),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;
  const address = config.address ?? 0x44;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let readChannels;
  if (config.emulation) {
    console.log('Emulation mode: generating ISL29125 readings without hardware');
    readChannels = async () => emulatedChannels();
  } else {
    console.log(
      `Initializing ISL29125 on I2C bus ${i2cBus}, address 0x${address.toString(16)}...`
    );
    const { ISL29125Driver } = require('./isl29125_driver');
    const driver = await ISL29125Driver.create(i2cBus, address);
    readChannels = () => driver.readChannels();
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  // The REST payload carries the colour, the discovery reply only the
  // illuminance — as in the Python node.
  const read = async () => {
    const { green, red, blue } = await readChannels();
    return deriveReading(green, red, blue);
  };
  const readDiscovery = async () => {
    const { green } = await readChannels();
    return { lux: Math.round(green * LUX_PER_COUNT) };
  };

  startDiscoveryListener('ISL29125', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('ISL29125', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
