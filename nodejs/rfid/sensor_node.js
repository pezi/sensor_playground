#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove 125KHz RFID Reader (Node.js)
 *
 * Implements the *push* variant of the Sensor Playground Sensor
 * Interface on single-board computers (Raspberry Pi & co.) with a Grove
 * 125KHz RFID Reader. The node reads RDM630-style frames from a
 * 9600-baud UART and pushes one JSON message ({"tag": "0F0024ADAB"})
 * per scanned EM4100 tag.
 *
 * Frame format (reader TX, jumper on UART mode — not Wiegand):
 *     STX 0x02 | 10 ASCII-hex data chars | 2 ASCII-hex checksum chars | ETX 0x03
 * The checksum byte is the XOR of the five data bytes. The reader
 * repeats the frame while a tag is held near the antenna, so the node
 * suppresses repeats of the same tag for REPEAT_SUPPRESS_MS.
 *
 * The port is configured with stty(1) and then read as a plain file
 * (like the CozIR node), so no native serial-port module is needed; the
 * reader is transmit-only, so frames simply arrive whenever a tag is
 * presented.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Set "emulation": true in config.json to generate plausible scans
 * without the reader hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { execFileSync } = require('child_process');
const fs = require('fs');

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');
const { RdmFrameParser } = require('./rfid_driver');

// Suppress repeats of the same tag while it is held near the antenna.
const REPEAT_SUPPRESS_MS = 2000;

const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

// -- Readers -----------------------------------------------------------------

/*
 * Reads RDM630-style frames from the reader UART and reports each
 * validated tag via onTag. Every received chunk is fed byte-by-byte
 * through the frame parser.
 */
function startSerialReader(portPath, onTag) {
  // Raw 9600 8N1 via stty; afterwards the UART behaves like a file.
  execFileSync('stty', ['-F', portPath, '9600', 'raw', '-echo']);
  const parser = new RdmFrameParser();
  const stream = fs.createReadStream(portPath);
  stream.on('data', (chunk) => {
    for (const byte of chunk) {
      const tag = parser.feed(byte);
      if (tag !== null) onTag(tag);
    }
  });
  stream.on('error', (err) => {
    console.log(`Error: serial port: ${err.message}`);
    process.exit(1);
  });
}

/*
 * Generates plausible RFID scans without hardware: one tag from a small
 * fixed pool every four to eight seconds.
 */
function startEmulatedReader(onTag) {
  const TAGS = ['0F0024ADAB', '0A0031B2C4', '03004F19AA', '1000C0FFEE'];
  const scheduleNext = () => {
    setTimeout(() => {
      onTag(TAGS[Math.floor(Math.random() * TAGS.length)]);
      scheduleNext();
    }, uniform(4000, 8000));
  };
  scheduleNext();
}

// -- Main --------------------------------------------------------------------

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const serialPort = config.serial_port || '/dev/serial0';

  if (config.emulation) {
    console.log('Emulation mode: generating RFID scans without hardware');
  } else {
    console.log(`Opening RFID reader on ${serialPort}...`);
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(config.api_key);
  startDiscoveryListener('RFID', hostname, WS_PORT, null);

  // Push each scanned tag, suppressing repeats of the same tag while it
  // is held near the antenna.
  let lastTag = null;
  let lastAt = 0;
  const onTag = (tag) => {
    if (tag === lastTag && Date.now() - lastAt < REPEAT_SUPPRESS_MS) return;
    console.log(`Tag: ${tag}`);
    broadcast({ tag });
    lastTag = tag;
    lastAt = Date.now();
  };

  if (config.emulation) {
    startEmulatedReader(onTag);
  } else {
    startSerialReader(serialPort, onTag);
  }
}

main();
