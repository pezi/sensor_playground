'use strict';
/*
 * Command parsing for the display node: the JSON commands the app pushes
 * over the WebSocket, a port of the Python node's handler.
 *
 * The Python node also accepts the same actions as chunked binary writes
 * over BLE; this port has no BLE transport, so that framing lives only in
 * the Rust port.
 */

const { FRAME_SIZE } = require('./ssd1306_driver');

/*
 * Execute one JSON command and return its ACK/NACK response. The app
 * matches the reply to its command by id, so an id is required even for a
 * command that fails validation.
 */
async function handleCommand(display, message) {
  let command;
  try {
    command = JSON.parse(message);
  } catch {
    console.log('Ignoring malformed command');
    return { id: null, ok: false, error: 'malformed JSON' };
  }
  if (command === null || typeof command !== 'object' || Array.isArray(command)) {
    return { id: null, ok: false, error: 'command must be an object' };
  }
  if (!Number.isInteger(command.id)) {
    return { id: null, ok: false, error: 'missing command id' };
  }
  const id = command.id;

  if (command.clear === true) {
    console.log('Command: clear');
    try {
      await display.clear();
    } catch (err) {
      return { id, ok: false, error: err.message };
    }
    return { id, ok: true };
  }

  if (typeof command.image !== 'string') {
    return { id, ok: false, error: 'unknown command' };
  }
  // Node's base64 decoder silently skips anything it cannot decode, so
  // validate the string first — the same strictness as the Python node's
  // b64decode(validate=True).
  if (!/^[A-Za-z0-9+/]*={0,2}$/.test(command.image) || command.image.length % 4 !== 0) {
    console.log('Ignoring command with invalid base64 image');
    return { id, ok: false, error: 'invalid base64' };
  }
  const data = Buffer.from(command.image, 'base64');
  if (data.length !== FRAME_SIZE) {
    console.log(`Ignoring image with ${data.length} bytes (need ${FRAME_SIZE})`);
    return { id, ok: false, error: `image has ${data.length} bytes; need ${FRAME_SIZE}` };
  }
  console.log('Command: image');
  try {
    await display.showBitmap(data);
  } catch (err) {
    return { id, ok: false, error: err.message };
  }
  return { id, ok: true };
}

module.exports = { handleCommand };
