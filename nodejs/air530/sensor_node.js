#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Air530 GPS (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a Grove GPS (Air530) module. Like
 * the CozIR the Air530 is not an I2C device: it continuously streams
 * NMEA-0183 sentences over a 9600-baud UART (serial). The node reads one
 * burst per request and parses the fix (port of the dart_periphery
 * NmeaParser, see serial_air530.dart):
 *
 * - GGA sentences (preferred): latitude, longitude, MSL altitude,
 *   satellites in use
 * - GLL sentences (fallback): latitude, longitude only
 * - Sentences with bad checksums are skipped
 *
 * The port is configured with stty(1) and then read as a plain file, so
 * no native serial-port module is needed.
 *
 * - HTTPS REST API on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   Wi-Fi with a warning (use the Python or Rust node for BLE).
 *
 * Until the module has a position fix the read yields an empty object —
 * not null — so the REST API answers 200 with a metadata-only body
 * instead of the shared transport's 503 (the Python node's allow_empty
 * behavior; the app then shows its "waiting for satellite fix" screen).
 *
 * Set "emulation": true in config.json to generate plausible readings
 * without the sensor hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { execFileSync } = require('child_process');
const fs = require('fs');

const { loadConfig, hostnameOr } = require('../common/config');
const { HTTPS_PORT, startDiscoveryListener, runRestServer } = require('../common/wifi');
const { parseNmea } = require('./air530_driver');

const READ_TIMEOUT_MS = 1000;
const BURST_BYTES = 512;

const round1 = (x) => Math.round(x * 10) / 10;
const round6 = (x) => Math.round(x * 1e6) / 1e6;
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

class Air530Serial {
  constructor(portPath) {
    // Raw 9600 8N1 via stty; afterwards the UART behaves like a file.
    execFileSync('stty', ['-F', portPath, '9600', 'raw', '-echo']);
    this._fd = fs.openSync(portPath, 'r');
    this._stream = fs.createReadStream('', { fd: this._fd, autoClose: false });
    this._buffer = Buffer.alloc(0);
    this._stream.on('data', (chunk) => {
      this._buffer = Buffer.concat([this._buffer, chunk]);
      if (this._wake) this._wake();
    });
    this._wake = null;
  }

  /*
   * Discard the stale buffer, then collect one ~1 Hz NMEA burst — like
   * the Python node's serial.read(512) with a 1 s timeout.
   */
  async readBlock() {
    this._buffer = Buffer.alloc(0);
    const deadline = Date.now() + READ_TIMEOUT_MS;
    while (Date.now() < deadline && this._buffer.length < BURST_BYTES) {
      await new Promise((resolve) => {
        this._wake = resolve;
        setTimeout(resolve, 50);
      });
      this._wake = null;
    }
    return this._buffer.toString('utf8').slice(0, BURST_BYTES);
  }
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const serialPort = config.serial_port || '/dev/serial0';
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating Air530 readings without hardware');
    // A receiver at St. Stephen's Cathedral in Vienna (48.2085 N,
    // 16.3730 E): the position performs a tiny random walk around the
    // base coordinate, the altitude drifts slowly around 171 m MSL and
    // the satellite count varies between 4 and 12.
    let lat = 48.2085;
    let lon = 16.373;
    read = async () => {
      lat += uniform(-0.0001, 0.0001);
      lon += uniform(-0.0001, 0.0001);
      return {
        latitude: round6(lat),
        longitude: round6(lon),
        altitude: round1(171.0 + 5.0 * Math.sin(Date.now() / 1000 / 120.0)),
        satellites: 4 + Math.floor(Math.random() * 9),
      };
    };
  } else {
    console.log(`Initializing Air530 GPS on ${serialPort}...`);
    const serial = new Air530Serial(serialPort);
    let busy = Promise.resolve();
    read = () => {
      // Serialize reads: discovery and REST must not interleave bursts.
      // A fixless warm-up burst yields {} (200 metadata-only), not null
      // (503) — a GPS still warming up is not an error.
      const result = busy.then(async () => parseNmea(await serial.readBlock()) || {});
      busy = result.catch(() => {});
      return result;
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full || full.latitude === undefined) return {};
    const short = { lat: full.latitude, lon: full.longitude };
    if (full.altitude !== undefined) short.alt = full.altitude;
    if (full.satellites !== undefined) short.sats = full.satellites;
    return short;
  };
  startDiscoveryListener('AIR530', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('AIR530', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
