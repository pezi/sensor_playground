#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove Ultrasonic Ranger (Node.js)
 *
 * Implements the *push* variant of the Sensor Playground Sensor Interface
 * with a Grove Ultrasonic Ranger (40 kHz sonar, 2 cm - 3.5 m). Like the
 * VL53L0X node it measures continuously and pushes one JSON message
 * ({"distance": <mm>}, null when no echo returns) whenever the distance
 * changes, or at least once per second as a heartbeat.
 *
 * EMULATION ONLY. The Grove ranger multiplexes trigger and echo on one
 * SIG pin, and the echo pulse must be timed with kernel edge timestamps —
 * one millisecond of pulse error is 17 cm of distance error, and no
 * Node.js GPIO path delivers those timestamps (a JavaScript polling loop
 * adds milliseconds of jitter). With "emulation": false the node prints
 * an error and exits; use the Python, Go or Rust node for the real
 * sensor.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Usage:
 *     cp config.example.json config.json   # set "emulation": true
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');

// -- Publish policy -----------------------------------------------------------

const MEASURE_INTERVAL_MS = 100;
const HEARTBEAT_MS = 1000;
// Larger than the VL53L0X's delta because a sonar reading jitters a little.
const MIN_DELTA_MM = 5;

// -- Emulated measurement -----------------------------------------------------

/*
 * Generates plausible ranger readings without hardware: a target sweeping
 * back and forth between 200 and 2000 mm (20 s period), occasionally
 * leaving the measuring range (null = no echo).
 */
function emulatedReadMm() {
  if (Math.random() < 0.02) return null;
  const phase = ((Date.now() / 1000) % 20.0) / 20.0;
  return Math.round(200 + 1800 * (1 - Math.abs(2 * phase - 1)));
}

// -- Measure loop -------------------------------------------------------------

/** Measures continuously; publishes on change, range flip, or heartbeat. */
function startMeasureLoop(readMm, publish) {
  let lastSent = null;
  let everPublished = false;
  let lastPublish = 0;

  setInterval(() => {
    const mm = readMm();
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
  }, MEASURE_INTERVAL_MS);
}

// -- Main --------------------------------------------------------------------

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'ULTRASONIC';

  if (!config.emulation) {
    console.log(
      'Error: the Node.js port cannot time the ultrasonic echo. The Grove\n' +
      'ranger multiplexes trigger and echo on one SIG pin, and the echo pulse\n' +
      'must be timed with kernel edge timestamps — one millisecond of pulse\n' +
      'error is 17 cm of distance error, and no Node.js GPIO path delivers\n' +
      'those timestamps. Use the Python, Go or Rust node for the real sensor,\n' +
      'or set "emulation": true in config.json to serve generated readings.'
    );
    process.exit(1);
  }
  console.log('Emulation mode: generating ranger readings without hardware');

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(config.api_key, null, null);
  startDiscoveryListener(sensorName, hostname, WS_PORT, null);
  startMeasureLoop(emulatedReadMm, broadcast);
}

main();
