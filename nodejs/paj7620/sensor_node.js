#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — PAJ7620 Grove Gesture (Node.js)
 *
 * Implements the *push* variant of the Sensor Playground Sensor Interface
 * on single-board computers (Raspberry Pi & co.) with a Grove Gesture
 * sensor (PAJ7620U2). The node polls the sensor over I2C and pushes one
 * JSON message ({"gesture": "forward"}) per detected gesture.
 *
 * The gesture strings match the app's Gesture enum names. Like the
 * grove.py driver the Python node uses, the nine basic gestures are
 * reported; the combined gestures (forwardBackward, rightLeft, ...) of
 * the ESP32 sketch are not detected.
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
const { PAJ7620Driver, GESTURES } = require('./paj7620_driver');

const POLL_INTERVAL_MS = 100;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// -- Sensor ------------------------------------------------------------------

/*
 * Generates plausible PAJ7620 gestures without hardware: one random
 * gesture from the gesture map every three to five seconds; polls in
 * between return no gesture (like the Python node).
 */
class EmulatedPAJ7620 {
  constructor() {
    this._nextAt = Date.now() + this._gap();
  }

  _gap() {
    return (3.0 + 2.0 * Math.random()) * 1000;
  }

  async readGesture() {
    if (Date.now() < this._nextAt) return null;
    this._nextAt = Date.now() + this._gap();
    const names = Object.values(GESTURES);
    return names[Math.floor(Math.random() * names.length)];
  }
}

// -- Main --------------------------------------------------------------------

/** Polls the sensor and pushes each detected gesture. */
async function gestureLoop(sensor, publish) {
  for (;;) {
    let name;
    try {
      name = await sensor.readGesture();
    } catch (err) {
      console.log(`Sensor read failed: ${err.message || err}`);
      process.exit(1);
    }
    if (name !== null) {
      console.log(`Gesture: ${name}`);
      publish({ gesture: name });
    }
    await sleep(POLL_INTERVAL_MS);
  }
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus === undefined ? 1 : config.i2c_bus;

  let sensor;
  if (config.emulation) {
    console.log('Emulation mode: generating PAJ7620 gestures without hardware');
    sensor = new EmulatedPAJ7620();
  } else {
    console.log(`Initializing PAJ7620 gesture sensor on /dev/i2c-${i2cBus}...`);
    try {
      sensor = await PAJ7620Driver.create(i2cBus);
    } catch (err) {
      console.log(`Error: ${err.message || err}`);
      process.exit(1);
    }
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(config.api_key, null, null);
  startDiscoveryListener('PAJ7620', hostname, WS_PORT, null);
  await gestureLoop(sensor, broadcast);
}

main();
