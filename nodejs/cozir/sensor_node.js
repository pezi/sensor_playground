#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — CozIR CO2 Sensor (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a CozIR-A sensor (temperature,
 * humidity, CO2). Unlike the other environment sensors the CozIR is not
 * an I2C device: it talks a simple ASCII command protocol over a
 * 9600-baud UART (serial).
 *
 * Protocol (see the Python node and dart_periphery's serial_cozir.dart):
 *     M 4164\r\n   select humidity, temperature and CO2 output fields
 *     K 2\r\n      polling mode
 *     Q\r\n        request one measurement:
 *                  "H 00495 T 01234 Z 06399" -> 49.5 %RH, 23.4 degC, 639.9 ppm
 *
 * The port is configured with stty(1) and then read/written as a plain
 * file, so no native serial-port module is needed.
 *
 * - HTTPS REST API on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   Wi-Fi with a warning (use the Python or Rust node for BLE).
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

// One measurement line: "H 00495 T 01234 Z 06399" (leading space may occur).
const MEASUREMENT_PATTERN = /H\s+(\d+)\s+T\s+(\d+)\s+Z\s+(\d+)/;

const READ_TIMEOUT_MS = 1000;

const round1 = (x) => Math.round(x * 10) / 10;
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

class CozirSerial {
  constructor(portPath) {
    // Raw 9600 8N1 via stty; afterwards the UART behaves like a file.
    execFileSync('stty', ['-F', portPath, '9600', 'raw', '-echo']);
    this._fd = fs.openSync(portPath, 'r+');
    this._stream = fs.createReadStream('', { fd: this._fd, autoClose: false });
    this._buffer = Buffer.alloc(0);
    this._stream.on('data', (chunk) => {
      this._buffer = Buffer.concat([this._buffer, chunk]);
      if (this._wake) this._wake();
    });
    this._wake = null;

    // Select the humidity, temperature and CO2 output fields, then switch
    // to polling mode (one measurement per Q command).
    fs.writeSync(this._fd, 'M 4164\r\n');
    fs.writeSync(this._fd, 'K 2\r\n');
    this._buffer = Buffer.alloc(0);
  }

  /** Send Q and wait for one measurement line (or time out -> null). */
  async poll() {
    this._buffer = Buffer.alloc(0);
    fs.writeSync(this._fd, 'Q\r\n');

    const deadline = Date.now() + READ_TIMEOUT_MS;
    while (Date.now() < deadline) {
      const text = this._buffer.toString('utf8');
      if (text.includes('\n') || text.length >= 64) return text;
      await new Promise((resolve) => {
        this._wake = resolve;
        setTimeout(resolve, 50);
      });
      this._wake = null;
    }
    return this._buffer.toString('utf8');
  }
}

function parseMeasurement(raw) {
  const match = MEASUREMENT_PATTERN.exec(raw || '');
  if (!match) return null;
  return {
    temperature: round1((parseInt(match[2], 10) - 1000) / 10.0),
    humidity: round1(parseInt(match[1], 10) / 10.0),
    co2: round1(parseInt(match[3], 10) / 10.0),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const serialPort = config.serial_port || '/dev/serial0';
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating CozIR readings without hardware');
    // A quiet indoor room: slow sines for temperature and humidity, a
    // bounded random walk between 400 and 1500 ppm for the CO2.
    let co2 = 600.0;
    read = async () => {
      const t = Date.now() / 1000;
      co2 = Math.min(Math.max(co2 + uniform(-15, 15), 400.0), 1500.0);
      return {
        temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0) + uniform(-0.1, 0.1)),
        humidity: round1(45.0 + 8.0 * Math.sin(t / 97.0) + uniform(-0.5, 0.5)),
        co2: round1(co2),
      };
    };
  } else {
    console.log(`Initializing CozIR sensor on ${serialPort}...`);
    const serial = new CozirSerial(serialPort);
    let busy = Promise.resolve();
    read = () => {
      // Serialize polls: discovery and REST must not interleave Q commands.
      const result = busy.then(async () => parseMeasurement(await serial.poll()));
      busy = result.catch(() => {});
      return result;
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    return { temp: full.temperature, hum: full.humidity, co2: full.co2 };
  };
  startDiscoveryListener('COZIR', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('COZIR', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
