#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove Capacitive Moisture Sensor (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers with a Grove Capacitive Moisture Sensor (Corrosion-Resistant)
 * — an analog probe whose output voltage falls as the soil gets wetter.
 * It reports the soil moisture as a percentage (JSON key `moisture`)
 * mapped linearly between the two calibration points in config.json (raw
 * ADC when dry vs. when wet), alongside the raw reading (`adc`/`adcMax`)
 * for calibrating them.
 *
 * The Raspberry Pi has no analog input, so the probe is read through
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
const HAT_ADC_MAX = 4095; // full scale of the hat's 12-bit ADC

// Averaging window; a single ADC read of the probe is noisy.
const SAMPLE_COUNT = 4;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Maps a raw ADC reading onto 0-100 % between the dry and wet calibration
// points (the probe's output falls as the soil gets wetter).
const percent = (raw, adcDry, adcWet) => {
  const span = adcDry - adcWet;
  const p = (100 * (adcDry - raw)) / span;
  return Math.round(Math.min(100, Math.max(0, p)));
};

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const hatType = config.hat_type || 'grove';
  const pin = config.pin ?? 0;
  const i2cBus = config.i2c_bus ?? 1;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';
  let adcDry = config.adc_dry ?? 2600;
  let adcWet = config.adc_wet ?? 1100;

  let readRaw;
  if (config.emulation) {
    console.log('Emulation mode: generating MOISTURE readings without hardware');
    // A watering cycle: the moisture slowly dries from ~85 % down to
    // ~25 % and jumps back up, on a few-minute loop for easy demoing.
    // Like the Python node, the emulation ignores the calibration config
    // and uses the 12-bit defaults.
    adcDry = 2600;
    adcWet = 1100;
    readRaw = async () => {
      const t = Date.now() / 1000;
      const cycle = (t % 300) / 300; // 0 -> 1 over five minutes
      const pct = 85 - 60 * cycle + 2 * Math.sin(t / 3);
      return Math.round(adcDry - ((adcDry - adcWet) * pct) / 100);
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
    if (adcDry === adcWet) {
      console.log('Error: adc_dry and adc_wet must differ (calibrate!)');
      process.exit(1);
    }
    console.log(`Initializing moisture probe on grove hat, channel ${pin} (dry=${adcDry}, wet=${adcWet})...`);
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
    // Averages the raw 12-bit ADC value [0-4095] of the channel; SMBus
    // words are little-endian.
    readRaw = async () => {
      let total = 0;
      for (let i = 0; i < SAMPLE_COUNT; i++) {
        total += await bus.readWord(HAT_I2C_ADDRESS, HAT_ADC_BASE + pin);
        await sleep(2);
      }
      return Math.round(total / SAMPLE_COUNT);
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const read = async () => {
    const raw = await readRaw();
    // The raw reading and its full scale help calibrate adc_dry/adc_wet.
    return { moisture: percent(raw, adcDry, adcWet), adc: raw, adcMax: HAT_ADC_MAX };
  };
  // Short-key reading for the discovery reply.
  startDiscoveryListener('MOISTURE', hostname, HTTPS_PORT, async () => ({
    moist: percent(await readRaw(), adcDry, adcWet),
  }));
  runRestServer('MOISTURE', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
