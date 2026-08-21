#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground EEPROM Node — AT24C128 (Node.js)
 *
 * Implements the *actuator* variant of the Sensor Playground Sensor
 * Interface on single-board computers (Raspberry Pi & co.) with an
 * AT24C128 serial EEPROM (128 Kbit / 16 KB, I2C address 0x50). The app
 * stores a short text on the chip and reads it back at any time; the
 * text survives power cycles of both ends.
 *
 * Like the LED the node is the single source of truth: after a write it
 * reads the chip back and reports the *stored* text, so a failed write
 * cannot leave the app showing a text the chip never held.
 *
 *     app -> node   {"write": "Hello"}   store the text on the chip
 *                   {"read": true}       re-read the chip and push
 *     node -> app   {"text": "Hello"}    stored text (on connect and
 *                                        after every write/read, read
 *                                        from the chip)
 *
 * EEPROM layout (offset 0): magic 'S' 'P', u16 big-endian text length
 * (max 512 bytes), then the UTF-8 text. A chip without the magic (e.g.
 * factory-fresh, all 0xFF) reads as an empty text.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Set "emulation": true in config.json to run without the chip (the text
 * then lives in memory only).
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');
const { TEXT_MAX_BYTES, At24c128Eeprom, EmulatedEeprom } = require('./at24c128_driver');

// Milliseconds between pending-publication polls (commands only mark the
// state dirty; this loop is what puts it on the wire).
const POLL_INTERVAL_MS = 50;

// -- Stored-text state -------------------------------------------------------

/*
 * Owns the stored text — the single source of truth this node publishes.
 * Every command marks the state as pending publication, even one that
 * does not change it: the published text is always a fresh read-back, so
 * the app renders what the chip actually holds.
 */
class EepromController {
  constructor(eeprom) {
    this._eeprom = eeprom;
    this.text = '';
    this._pending = false;
    this._queue = Promise.resolve();
  }

  /** Reads the initial text before the node starts serving. */
  async start() {
    await this._reread();
  }

  /** Stores [textBytes] and re-reads the chip. */
  write(textBytes) {
    this._enqueue(async () => {
      if (textBytes.length > TEXT_MAX_BYTES) {
        console.log(`Rejecting write of ${textBytes.length} bytes (max ${TEXT_MAX_BYTES})`);
      } else {
        await this._eeprom.writeText(textBytes);
      }
      await this._reread();
    });
  }

  /** Re-reads the chip and marks the text for publication. */
  read() {
    this._enqueue(() => this._reread());
  }

  async _reread() {
    this.text = await this._eeprom.readText();
    this._pending = true;
  }

  /*
   * Commands arrive from the WebSocket dispatch but the chip access is
   * asynchronous, so they are serialized through one chain — two
   * commands must not interleave their I2C transactions. A failed access
   * keeps the last known text rather than blanking the app.
   */
  _enqueue(step) {
    this._queue = this._queue.then(step).catch((err) => {
      console.log(`EEPROM access failed: ${err.message}`);
      this._pending = true;
    });
  }

  /** Returns true once after each command, clearing the pending flag. */
  takePending() {
    const pending = this._pending;
    this._pending = false;
    return pending;
  }
}

// -- Commands ----------------------------------------------------------------

/** Executes one JSON command pushed by the app over the WebSocket. */
function handleJsonCommand(controller, message) {
  let command;
  try {
    command = JSON.parse(message);
  } catch {
    console.log('Ignoring malformed command');
    return;
  }
  if (command.read === true) {
    console.log('Command: read');
    controller.read();
    return;
  }
  if (typeof command.write !== 'string') {
    console.log("Ignoring command without a string 'write'");
    return;
  }
  console.log(`Command: write ${[...command.write].length} characters`);
  controller.write(Buffer.from(command.write, 'utf8'));
}

// -- Text loop ---------------------------------------------------------------

/*
 * Publishes the stored text whenever a command marked it pending. The
 * command handlers run inside the WebSocket dispatch and only mutate the
 * controller; this loop puts the result on the wire.
 */
function startTextLoop(controller, publish) {
  setInterval(() => {
    if (controller.takePending()) {
      console.log(`text: ${JSON.stringify(controller.text)}`);
      publish({ text: controller.text });
    }
  }, POLL_INTERVAL_MS);
}

// -- Main --------------------------------------------------------------------

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'AT24C128';
  const i2cBus = config.i2c_bus ?? 1;
  const i2cAddress = parseInt(String(config.i2c_address ?? '0x50'), 16);

  let eeprom;
  if (config.emulation) {
    console.log('Emulation mode: storing the text in memory without hardware');
    eeprom = new EmulatedEeprom();
  } else {
    console.log(`Opening AT24C128 on i2c bus ${i2cBus}...`);
    eeprom = await At24c128Eeprom.create(i2cBus, i2cAddress);
  }

  const controller = new EepromController(eeprom);
  await controller.start();

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(
    config.api_key,
    // The chip holds state: a client that just connected gets the stored
    // text instead of an empty field.
    (send) => send({ text: controller.text }),
    (message) => handleJsonCommand(controller, message)
  );
  startDiscoveryListener(sensorName, hostname, WS_PORT, null);
  startTextLoop(controller, broadcast);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
