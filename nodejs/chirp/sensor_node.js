#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Chirp I2C Soil Moisture Sensor (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers with a Catnip Electronics I2C Soil Moisture Sensor (the
 * "Chirp" sensor, default I2C address 0x20). It reports the soil
 * moisture as a percentage (JSON key `moisture`, mapped linearly between
 * the two capacitance calibration points in config.json), the soil
 * temperature (`temperature`) and the ambient light level (`light`, raw
 * brightness counts, higher = brighter), alongside the raw capacitance
 * (`cap`) for calibrating.
 *
 * The chip measures light by timing a phototransistor discharge, which
 * takes up to three seconds — so the light value is harvested from a
 * measurement started on an earlier read, and the `light` key is absent
 * until the first one completes (a few seconds after start).
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
const {
  ChirpDriver,
  moisturePercent,
  decodeTemperature,
  lightCounts,
} = require('./chirp_driver');

/*
 * Emulation: a watering cycle for the moisture, a steady room
 * temperature and a slow day/night curve for the light. Like the Python
 * node, the emulation ignores the configured calibration points and uses
 * the defaults.
 */
const EMU_CAP_DRY = 290;
const EMU_CAP_WET = 520;

function emulatedRead() {
  const t = Date.now() / 1000;
  const cycle = (t % 300) / 300; // rewatered every five minutes
  const pct = 85 - 60 * cycle + 2 * Math.sin(t / 3);
  const capacitance = Math.round(EMU_CAP_DRY + ((EMU_CAP_WET - EMU_CAP_DRY) * pct) / 100);
  const rawTemp = Math.round(10 * (21.5 + 1.5 * Math.sin(t / 60)));
  const rawLight = Math.round(20000 + 15000 * Math.sin(t / 120));
  return {
    moisture: moisturePercent(capacitance, EMU_CAP_DRY, EMU_CAP_WET),
    temperature: decodeTemperature(rawTemp),
    cap: capacitance,
    // The emulated measurement finishes instantly, so unlike the real
    // sensor the key is present from the first read.
    light: lightCounts(rawLight),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;
  const address = config.address ?? 0x20;
  const capDry = config.cap_dry ?? 290;
  const capWet = config.cap_wet ?? 520;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating CHIRP readings without hardware');
    read = async () => emulatedRead();
  } else {
    if (capDry === capWet) {
      console.log('Error: cap_dry and cap_wet must differ (calibrate!)');
      process.exit(1);
    }
    console.log(`Initializing Chirp sensor on I2C bus ${i2cBus} (dry=${capDry}, wet=${capWet})...`);
    const driver = await ChirpDriver.create(i2cBus, address);
    read = async () => {
      const r = await driver.read();
      const reading = {
        moisture: moisturePercent(r.capacitance, capDry, capWet),
        temperature: r.temperature,
        // The raw capacitance helps calibrate cap_dry/cap_wet.
        cap: r.capacitance,
      };
      if (r.light !== null) reading.light = r.light;
      return reading;
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  // Short-key reading for the discovery reply.
  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    const short = { moist: full.moisture, temp: full.temperature };
    if (full.light !== undefined) short.light = full.light;
    return short;
  };
  startDiscoveryListener('CHIRP', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('CHIRP', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
