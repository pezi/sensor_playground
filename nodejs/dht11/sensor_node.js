#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — DHT11 / Grove Temperature & Humidity (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a DHT11 sensor (temperature,
 * humidity) — the blue Grove Temperature & Humidity Sensor module. The
 * DHT22 (Grove "Pro" module, white) speaks the same single-wire protocol
 * with better resolution and has its own node in ../dht22/.
 *
 * EMULATION ONLY. The single-wire protocol is timing-critical: a bit is
 * a 26-28 µs (0) or 70 µs (1) high phase, so the whole frame has to be
 * decoded from kernel edge timestamps — no Node.js GPIO path delivers
 * those timestamps (a JavaScript polling loop adds milliseconds of
 * jitter, thousands of times the pulse difference). With
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

const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a comfortable indoor climate — temperature and humidity
 * drift on slow sines with different periods around 22 °C / 45 %RH,
 * quantised to the DHT11's whole-degree / whole-percent resolution (like
 * the Python node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  return {
    temperature: Math.round(22.0 + 2.0 * Math.sin(t / 60.0)),
    humidity: Math.round(45.0 + 8.0 * Math.sin(t / 97.0) + uniform(-0.5, 0.5)),
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
  const sensorName = config.sensor_name || 'DHT11';
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  if (!config.emulation) {
    console.log(
      'Error: the Node.js port cannot read the DHT single-wire protocol. A\n' +
      'bit is a 26-28 µs (0) or 70 µs (1) pulse, so the frame has to be\n' +
      'decoded from kernel edge timestamps, and no Node.js GPIO path\n' +
      'delivers those timestamps (a JavaScript polling loop adds\n' +
      'milliseconds of jitter, thousands of times the pulse difference).\n' +
      'Use the Python, Go or Rust node for the real sensor, or set\n' +
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
