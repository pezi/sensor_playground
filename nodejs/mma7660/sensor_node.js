#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — MMA7660 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a Grove 3-Axis Digital
 * Accelerometer ±1.5g (MMA7660FC). The raw axes are converted into
 * roll / pitch angles plus the total acceleration magnitude (g-force).
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133 —
 *   the app streams accelerometers rather than polling them
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

// The app streams accelerometers instead of polling them, so readings are
// pushed at the same 250 ms cadence the BLE transport and the ESP32 use.
const PUSH_INTERVAL_MS = 250;

const round1 = (x) => Math.round(x * 10) / 10;
const round2 = (x) => Math.round(x * 100) / 100;
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a gently rocking, near-level board — roll and pitch follow
 * slow sines with different periods and the g-force stays around 1 g.
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  return {
    roll: round1(8.0 * Math.sin(t / 7.0) + uniform(-0.3, 0.3)),
    pitch: round1(5.0 * Math.sin(t / 11.0) + uniform(-0.3, 0.3)),
    gforce: round2(1.0 + uniform(-0.02, 0.02)),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating MMA7660 readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing MMA7660 sensor on /dev/i2c-${i2cBus}...`);
    const { MMA7660Driver } = require('./mma7660_driver');
    const driver = await MMA7660Driver.create(i2cBus);
    read = async () => {
      const r = await driver.read();
      return { roll: round1(r.roll), pitch: round1(r.pitch), gforce: round2(r.gforce) };
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(config.api_key);

  // The discovery reply carries the same full keys as the WebSocket
  // payload (the Python node reuses read() for discovery); a failed read
  // falls back to the identity-only reply.
  const readDiscovery = async () => {
    try {
      return await read();
    } catch {
      return {};
    }
  };
  startDiscoveryListener('MMA7660', hostname, WS_PORT, readDiscovery);

  // Push a reading every PUSH_INTERVAL_MS, like the ESP32 sketch.
  setInterval(async () => {
    let data = null;
    try {
      data = await read();
    } catch (err) {
      console.log(`Sensor read failed: ${err.message}`);
    }
    if (data) broadcast({ sensor: 'MMA7660', host: hostname, ...data });
  }, PUSH_INTERVAL_MS);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
