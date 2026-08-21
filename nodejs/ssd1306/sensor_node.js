#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Display Node — SSD1306 128x64 OLED (Node.js)
 *
 * Implements the *display* variant of the Sensor Playground Sensor
 * Interface on single-board computers (Raspberry Pi & co.) with an SSD1306
 * I2C OLED. Unlike sensor nodes this node consumes data: the app pushes
 * one JSON command per action over the WebSocket and the node draws it on
 * the panel.
 *
 *     {"id": 1, "image": "<base64>"}   show a bitmap (1024 bytes, see below)
 *     {"id": 2, "clear": true}         blank the display
 *
 * The node acknowledges each applied command with the matching id, or
 * returns an error ACK when validation or the display write fails.
 *
 * Bitmap format (matches the dart_periphery SSD1306 example): 128x64
 * pixels, 1 bit per pixel, packed horizontally row by row — 16 bytes per
 * row, MSB of each byte is the leftmost pixel (the
 * https://javl.github.io/image2cpp/ "horizontal" byte orientation). The
 * node transposes this into the SSD1306 native page format before writing
 * it over I2C.
 *
 * - WebSocket server (ws://) on port 9132, X-Api-Key checked on the
 *   handshake + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python node for BLE).
 *
 * Set "emulation": true to run without a panel.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');
const { EmulatedDisplay } = require('./ssd1306_driver');
const { handleCommand } = require('./commands');

function installShutdownHandlers(display, signalTarget = process, exit = (code) => process.exit(code)) {
  let stopping = false;
  const shutdown = () => {
    if (stopping) return;
    stopping = true;
    Promise.resolve()
      .then(() => display.clear())
      .catch((err) => console.log(`Display clear failed: ${err.message}`))
      .finally(() => {
        console.log('Stopped.');
        exit(0);
      });
  };

  signalTarget.once('SIGINT', shutdown);
  signalTarget.once('SIGTERM', shutdown);
  return shutdown;
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;

  let display;
  if (config.emulation) {
    console.log('Emulation mode: reporting frames without hardware');
    display = new EmulatedDisplay();
  } else {
    // The config carries the address as a hex string, like the Python node.
    const address = parseInt(String(config.i2c_address ?? '0x3C'), 16);
    if (Number.isNaN(address)) {
      throw new Error(`i2c_address ${config.i2c_address} is not a hex address like "0x3C"`);
    }
    console.log('Initializing SSD1306 display...');
    const { SSD1306Driver } = require('./ssd1306_driver');
    display = await SSD1306Driver.create(i2cBus, address);
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  // Blank the panel rather than leaving a stale image burning.
  installShutdownHandlers(display);

  startWsPushServer(config.api_key, null, (message) => handleCommand(display, message));
  startDiscoveryListener('SSD1306', hostname, WS_PORT, null);
}

if (require.main === module) {
  main().catch((err) => {
    console.log(`Error: ${err.message}`);
    process.exit(1);
  });
}

module.exports = { installShutdownHandlers };
