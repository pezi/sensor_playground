#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — VL53L0X (Node.js)
 *
 * Implements the *push* variant of the Sensor Playground Sensor Interface
 * on single-board computers (Raspberry Pi & co.) with a VL53L0X
 * time-of-flight distance sensor. The node measures continuously and
 * pushes one JSON message ({"distance": <mm>}, null when out of range)
 * whenever the distance changes, or at least once per second as a
 * heartbeat.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Set "emulation": true in config.json to generate plausible readings
 * without the sensor hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');

// -- Publish policy -----------------------------------------------------------

const MEASURE_INTERVAL_MS = 100;
const HEARTBEAT_MS = 1000;
const MIN_DELTA_MM = 3;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// -- Sensor ------------------------------------------------------------------

/*
 * Returns an async read() -> distance in mm, or null when no target is in
 * range. Emulation: a target sweeping back and forth between 100 and
 * 1200 mm (20 s period), occasionally leaving the measuring range (like
 * the Python node).
 */
async function makeReader(config) {
  if (config.emulation) {
    return async () => {
      if (Math.random() < 0.02) return null;
      const phase = ((Date.now() / 1000) % 20.0) / 20.0;
      return Math.round(100 + 1100 * (1 - Math.abs(2 * phase - 1)));
    };
  }
  const { VL53L0XDriver } = require('./vl53l0x_driver');
  const driver = await VL53L0XDriver.create(config.i2c_bus ?? 1);
  return async () => {
    const mm = await driver.readRange();
    // The VL53L0X reports ~8190 mm when no target is in range.
    return mm > 0 && mm < 8000 ? mm : null;
  };
}

// -- Measure loop -------------------------------------------------------------

/** Measure continuously; publish on change, range flip, or heartbeat. */
async function measureLoop(read, publish) {
  let lastSent = null;
  let lastPublish = 0;
  let everPublished = false;

  for (;;) {
    let mm;
    try {
      mm = await read();
    } catch (err) {
      console.log(`I2C read failed: ${err.message}`);
      await sleep(MEASURE_INTERVAL_MS);
      continue;
    }
    const now = Date.now();
    const changed =
      !everPublished ||
      (mm === null) !== (lastSent === null) ||
      (mm !== null && lastSent !== null && Math.abs(mm - lastSent) >= MIN_DELTA_MM);
    if (changed || now - lastPublish >= HEARTBEAT_MS) {
      publish({ distance: mm });
      everPublished = true;
      lastSent = mm;
      lastPublish = now;
    }
    await sleep(MEASURE_INTERVAL_MS);
  }
}

// -- Main --------------------------------------------------------------------

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);

  if (config.emulation) {
    console.log('Emulation mode: generating VL53L0X readings without hardware');
  } else {
    console.log('Initializing VL53L0X sensor...');
  }
  const read = await makeReader(config);

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(config.api_key);
  startDiscoveryListener('VL53L0X', hostname, WS_PORT, null);
  await measureLoop(read, broadcast);
}

main();
