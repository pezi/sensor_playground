#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove IMU 10DOF (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a Grove IMU 10DOF board
 * (MPU9250 + BMP280). The MPU9250 axes are reduced to roll / pitch /
 * compass heading angles plus the total acceleration magnitude
 * (g-force); the BMP280 adds temperature and barometric pressure.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 *   — the app streams motion sensors rather than polling them
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

const SENSOR_NAME = 'IMU10DOF';

// The app streams motion sensors instead of polling them, so readings are
// pushed at the same 250 ms cadence the BLE transport and the ESP32 use.
const PUSH_INTERVAL_MS = 250;

const round1 = (x) => Math.round(x * 10) / 10;
const round2 = (x) => Math.round(x * 100) / 100;
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a gently rocking, near-level board — roll and pitch follow
 * slow sines with different periods, the g-force stays around 1 g and
 * the heading sweeps a full circle every two minutes. The BMP280 side
 * reports room temperature and sea-level pressure, each drifting slowly
 * (like the Python node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  return {
    temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0)),
    pressure: round2(1013.0 + 3.0 * Math.sin(t / 300.0)),
    roll: round1(8.0 * Math.sin(t / 7.0) + uniform(-0.3, 0.3)),
    pitch: round1(5.0 * Math.sin(t / 11.0) + uniform(-0.3, 0.3)),
    heading: round1((t * 3) % 360),
    gforce: round2(1.0 + uniform(-0.02, 0.02)),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating IMU10DOF readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing IMU 10DOF on /dev/i2c-${i2cBus}...`);
    const { IMU10DOFDriver } = require('./imu10dof_driver');
    const driver = await IMU10DOFDriver.create(i2cBus);
    read = async () => {
      try {
        const r = await driver.read();
        return {
          temperature: round1(r.temperature),
          pressure: round2(r.pressure),
          roll: round1(r.roll),
          pitch: round1(r.pitch),
          heading: round1(r.heading),
          gforce: round2(r.gforce),
        };
      } catch (err) {
        console.log(`Sensor read failed: ${err.message}`);
        return null;
      }
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return {
      temp: full.temperature,
      press: full.pressure,
      roll: full.roll,
      pitch: full.pitch,
      heading: full.heading,
      gforce: full.gforce,
    };
  };

  const { broadcast } = startWsPushServer(config.api_key);
  startDiscoveryListener(SENSOR_NAME, hostname, WS_PORT, readDiscovery);

  // Push a reading every PUSH_INTERVAL_MS, like the ESP32 sketch.
  setInterval(async () => {
    const data = await read();
    if (data) broadcast({ sensor: SENSOR_NAME, host: hostname, ...data });
  }, PUSH_INTERVAL_MS);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
