#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove Dust Sensor / Shinyei PPD42NS (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface with a Grove Dust
 * Sensor (Shinyei PPD42NS). The sensor pulls its output pin LOW while
 * particles scatter light inside its chamber; the node accumulates that
 * low-pulse occupancy (LPO) over 30-second windows and converts the
 * ratio into a particle concentration in pcs/0.01cf using the Nafis
 * curve, reported under the JSON key `dust`.
 *
 * EMULATION ONLY. The PPD42NS signals particles as 10-90 ms LOW pulses
 * whose occupancy must be accumulated from kernel edge timestamps — no
 * Node.js GPIO path delivers those timestamps (a JavaScript polling loop
 * adds milliseconds of jitter per edge, a large error on a 10 ms pulse).
 * With "emulation": false the node prints an error and exits; use the
 * Python, Go or Rust node for the real sensor.
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
 * Emulation: indoor air over the day — the concentration follows a slow
 * sine between roughly 100 and 700 pcs/0.01cf, deliberately crossing
 * several Dylos air-quality bands, with a little measurement jitter,
 * never below zero (like the Python node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  const concentration = 400 + 300 * Math.sin(t / 180.0) + uniform(-40, 40);
  return { dust: round1(Math.max(0.0, concentration)) };
}

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  if (!config.emulation) {
    console.log(
      'Error: the Node.js port cannot time the PPD42NS dust pulses. The\n' +
      'sensor signals particles as 10-90 ms LOW pulses whose occupancy must\n' +
      'be accumulated from kernel edge timestamps, and no Node.js GPIO path\n' +
      'delivers those timestamps (a JavaScript polling loop adds\n' +
      'milliseconds of jitter per edge). Use the Python, Go or Rust node\n' +
      'for the real sensor, or set "emulation": true in config.json to\n' +
      'serve generated readings.'
    );
    process.exit(1);
  }
  console.log('Emulation mode: generating PPD42NS readings without hardware');

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const read = async () => emulatedRead();
  // The dust value uses the same key on both payloads.
  startDiscoveryListener('PPD42NS', hostname, HTTPS_PORT, read);
  runRestServer('PPD42NS', config.api_key, hostname, read, sslCert, sslKey);
}

main();
