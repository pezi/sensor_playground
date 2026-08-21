#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — SHT11 / Sensirion SHT1x (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a Sensirion SHT1x sensor
 * (temperature, humidity) — the classic SHT10 / SHT11 / SHT15 family. The
 * chips differ only in calibration accuracy and speak the same
 * proprietary two-wire protocol (SCK + bidirectional DATA); it resembles
 * I2C but is NOT I2C — the sensor cannot share an I2C bus. Set
 * "sensor_name" in config.json to the chip on your board so the app shows
 * the right name.
 *
 * EMULATION ONLY. The two-wire protocol is not timing-critical (the bus
 * is fully master-clocked), but it needs a bidirectional, open-drain DATA
 * line that keeps its state across hundreds of edges of one transaction:
 * released for a 1, actively pulled LOW for a 0, sampled while the sensor
 * answers. Node.js has no GPIO character-device binding on Debian 13
 * (/sys/class/gpio is gone and node-libgpiod does not build against
 * libgpiod 2.x), and the gpioset/gpioget CLIs release the line when they
 * exit — so the handshake collapses after the very first edge. With
 * "emulation": false the node prints an error and exits; use the Python,
 * Go or Rust node for the real sensor.
 *
 * - HTTPS REST API on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   Wi-Fi with a warning (use the Python or Rust node for BLE).
 *
 * Usage:
 *     cp config.example.json config.json   # set "emulation": true
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { HTTPS_PORT, startDiscoveryListener, runRestServer } = require('../common/wifi');

const round1 = (x) => Math.round(x * 10) / 10;

const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a comfortable indoor climate — temperature and humidity
 * drift on slow sines with different periods around 22 °C / 45 %RH, plus
 * a little measurement noise (like the Python node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  return {
    temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0) + uniform(-0.1, 0.1)),
    humidity: round1(45.0 + 8.0 * Math.sin(t / 97.0) + uniform(-0.5, 0.5)),
  };
}

/** Short-key readings for the UDP discovery response. */
function toDiscovery(full) {
  if (!full) return {};
  return { temp: full.temperature, hum: full.humidity };
}

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'SHT11';
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  if (!config.emulation) {
    console.log(
      'Error: the Node.js port cannot bit-bang the SHT1x two-wire protocol.\n' +
      'The DATA line is bidirectional and driven open-drain — released for\n' +
      'a 1, pulled LOW for a 0, sampled while the sensor answers — and that\n' +
      'state has to survive hundreds of clock edges of one transaction.\n' +
      'Node.js has no GPIO character-device binding on Debian 13\n' +
      '(/sys/class/gpio is gone, node-libgpiod does not build against\n' +
      'libgpiod 2.x), and the gpioset/gpioget CLIs release the line when\n' +
      'they exit, so the handshake collapses after the first edge. Use the\n' +
      'Python, Go or Rust node for the real sensor, or set\n' +
      '"emulation": true in config.json to serve generated readings.'
    );
    process.exit(1);
  }
  console.log(`Emulation mode: generating ${sensorName} readings without hardware`);

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const read = async () => emulatedRead();
  const readDiscovery = async () => toDiscovery(await read());
  startDiscoveryListener(sensorName, hostname, HTTPS_PORT, readDiscovery);
  runRestServer(sensorName, config.api_key, hostname, read, sslCert, sslKey);
}

main();
