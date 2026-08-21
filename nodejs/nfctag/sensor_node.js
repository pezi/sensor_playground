#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove NFC Tag (Node.js)
 *
 * Implements the *push* variant of the Sensor Playground Sensor Interface
 * on single-board computers (Raspberry Pi & co.) with a Grove NFC Tag — a
 * passive dual-interface EEPROM (ST M24LR64E-R, 8 KB). A phone or NFC
 * writer stores an NDEF message over the ISO 15693 RF interface; this
 * node reads the same memory over I2C, parses the first NDEF record and
 * pushes one JSON message whenever the content changes:
 *
 *     {"kind": "text", "value": "Hello"}
 *     {"kind": "uri",  "value": "https://seeed.cc"}
 *     {"kind": "data", "value": "DEADBEEF"}      (hex, truncated)
 *     {"kind": "empty"}
 *
 * Unlike the pure event sensors the tag holds state, so the current
 * content is also sent to every client right after it connects.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Set "emulation": true in config.json to cycle through generated
 * contents without the tag hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');
const { M24lr64Tag, EmulatedNfcTag } = require('./nfctag_driver');

// Milliseconds between EEPROM polls; an RF write shows up on the next poll.
const POLL_INTERVAL_MS = 1000;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Two contents are equal when kind and value match ('empty' has no value). */
function contentEquals(a, b) {
  return a !== null && b !== null && a.kind === b.kind && a.value === b.value;
}

/** Poll the tag and push the content whenever it changes. */
async function contentLoop(tag, state, publish) {
  for (;;) {
    let content = null;
    try {
      content = await tag.readContent();
    } catch (err) {
      console.log(`Tag read failed: ${err.message}`);
    }
    if (content !== null && !contentEquals(content, state.content)) {
      console.log(`Content: ${JSON.stringify(content)}`);
      state.content = content;
      publish(content);
    }
    await sleep(POLL_INTERVAL_MS);
  }
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;
  const i2cAddress = parseInt(String(config.i2c_address ?? '0x53'), 16);

  let tag;
  if (config.emulation) {
    console.log('Emulation mode: cycling NFC tag contents without hardware');
    tag = new EmulatedNfcTag();
  } else {
    console.log(`Opening M24LR64E on i2c bus ${i2cBus}...`);
    tag = await M24lr64Tag.create(i2cBus, i2cAddress);
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  // The tag holds state: a client that just connected gets the current
  // content instead of waiting for the next RF write.
  const state = { content: null };
  const { broadcast } = startWsPushServer(
    config.api_key,
    (send) => {
      if (state.content !== null) send(state.content);
    },
    null // push node: incoming messages are ignored
  );
  startDiscoveryListener('NFCTAG', hostname, WS_PORT, null);
  await contentLoop(tag, state, broadcast);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
