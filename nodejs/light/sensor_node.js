#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove Light Sensor (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers with a Grove Light Sensor — an analog photo-resistor
 * reporting a raw brightness value (higher = brighter) under the JSON
 * key `light`.
 *
 * The Raspberry Pi has no analog input, so the sensor is read through
 * the Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04, one 16-bit
 * register per channel). The Arduino-based hats the Python node also
 * supports ("nano", "grovePlus") are not implemented in this port.
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

const HAT_I2C_ADDRESS = 0x04; // Grove Base Hat (STM32F030 ADC)
const HAT_ADC_BASE = 0x10; // raw 12-bit value registers, one per channel

const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const hatType = config.hat_type || 'grove';
  const pin = config.pin ?? 0;
  const i2cBus = config.i2c_bus ?? 1;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating LIGHT readings without hardware');
    // Daylight through a window: a slow sine around a few hundred counts
    // with a little flicker, never below zero.
    read = async () => {
      const t = Date.now() / 1000;
      const raw = 400 + 350 * Math.sin(t / 120.0) + uniform(-15, 15);
      return { light: Math.max(0, Math.round(raw)) };
    };
  } else {
    if (hatType !== 'grove') {
      console.log(`Error: hat_type "${hatType}" is not supported in the Node.js port (only "grove"; use the Python node for Arduino-based hats)`);
      process.exit(1);
    }
    if (pin < 0 || pin > 7) {
      console.log(`Error: invalid channel ${pin} - valid range [0,7]`);
      process.exit(1);
    }
    console.log(`Initializing Grove Light Sensor on grove hat, channel ${pin}...`);
    let i2c;
    try {
      i2c = require('i2c-bus');
    } catch {
      console.log(
        "Error: the i2c-bus package is not installed. It is an *optional* npm\n" +
        "dependency (so a failed native build does not stop `npm install`).\n" +
        "In this node's folder run:\n" +
        "    sudo apt install -y build-essential python3\n" +
        "    npm install\n" +
        "and check the output for i2c-bus build errors. Or set\n" +
        '"emulation": true in config.json to run without hardware.'
      );
      process.exit(1);
    }
    const bus = await i2c.openPromisified(i2cBus);
    // Reads the raw 12-bit ADC value [0-4095]; SMBus words are little-endian.
    read = async () => ({ light: await bus.readWord(HAT_I2C_ADDRESS, HAT_ADC_BASE + pin) });
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  // The light value uses the same key on both payloads.
  startDiscoveryListener('LIGHT', hostname, HTTPS_PORT, async () => (await read()) || {});
  runRestServer('LIGHT', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
