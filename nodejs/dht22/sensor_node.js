#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — DHT22 / Grove Temperature & Humidity Pro
 * (Node.js)
 *
 * EMULATION ONLY. The DHT22's 26-70 µs single-wire pulses require kernel
 * edge timestamps; the available Node.js GPIO paths cannot deliver that
 * timing. Use the Python, Go or Rust node for real hardware.
 *
 * - HTTPS REST API on port 9132 + UDP discovery on port 9133
 * - BLE is not supported; a BLE configuration falls back to Wi-Fi
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { HTTPS_PORT, startDiscoveryListener, runRestServer } = require('../common/wifi');

const uniform = (lo, hi) => lo + Math.random() * (hi - lo);
const round1 = (value) => Math.round(value * 10) / 10;

function emulatedRead(t = Date.now() / 1000, noise = uniform(-0.5, 0.5)) {
  return {
    temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0)),
    humidity: round1(45.0 + 8.0 * Math.sin(t / 97.0) + noise),
  };
}

function toDiscovery(full) {
  if (!full) return {};
  return { temp: full.temperature, hum: full.humidity };
}

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'DHT22';
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  if (!config.emulation) {
    console.log(
      'Error: the Node.js port cannot read the DHT single-wire protocol. A\n' +
      'bit is a 26-28 µs (0) or 70 µs (1) pulse, and no Node.js GPIO path\n' +
      'provides the required kernel edge timestamps. Use the Python, Go or\n' +
      'Rust node for real hardware, or set "emulation": true in config.json.'
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

if (require.main === module) main();

module.exports = { emulatedRead, toDiscovery };
