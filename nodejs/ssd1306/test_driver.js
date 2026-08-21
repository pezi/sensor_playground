#!/usr/bin/env node
'use strict';
/*
 * Unit test: the bitmap transposition and the command handling must match
 * the Python node's. The expected values below were produced by running the
 * reference Python implementation on the same input.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');
const crypto = require('crypto');
const { EventEmitter } = require('events');

const { toNativeFormat, FRAME_SIZE, ROW_BYTES, HEIGHT, WIDTH } = require('./ssd1306_driver');
const { handleCommand } = require('./commands');
const { installShutdownHandlers } = require('./sensor_node');
const { _attachClient } = require('../common/ws_server');

// A protocol/socket error from one WebSocket client must be handled locally,
// otherwise EventEmitter terminates the complete sensor process.
{
  class FakeClient extends EventEmitter {
    send() {}
  }
  const client = new FakeClient();
  const clients = new Set();
  _attachClient(client, clients, null, null);
  assert.strictEqual(client.listenerCount('error'), 1, 'client errors must be handled');
  client.emit('error', new Error('invalid frame'));
  assert.ok(!clients.has(client), 'failed client must be removed');
}

// The top-left pixel is the MSB of the first byte in the app's format, and
// the LSB of the first column byte in the panel's.
{
  const data = Buffer.alloc(FRAME_SIZE);
  data[0] = 0x80;
  const native = toNativeFormat(data);
  assert.strictEqual(native[0], 0x01);
  assert.strictEqual(native.subarray(1).reduce((a, b) => a + b, 0), 0, 'only one pixel is lit');
}

// Row 7, x = 127: the last row of page 0 (the MSB of a column byte) at the
// last column, which is the last byte of the first page.
{
  const data = Buffer.alloc(FRAME_SIZE);
  data[7 * ROW_BYTES + 15] = 0x01;
  assert.strictEqual(toNativeFormat(data)[127], 0x80);
}

// All white stays all white; alternating rows become 0b01010101, since each
// column byte holds eight vertical pixels.
{
  const white = Buffer.alloc(FRAME_SIZE, 0xff);
  assert.ok(toNativeFormat(white).every((v) => v === 0xff), 'all-white frame');

  const stripes = Buffer.alloc(FRAME_SIZE);
  for (let row = 0; row < HEIGHT; row += 2) {
    for (let i = 0; i < ROW_BYTES; i++) stripes[row * ROW_BYTES + i] = 0xff;
  }
  assert.ok(toNativeFormat(stripes).every((v) => v === 0x55), 'striped frame');
}

// A full pseudo-random frame, checked against the reference implementation
// by hash — the uniform patterns above cannot catch a transposition that
// scrambles bytes within a page.
{
  const data = Buffer.alloc(FRAME_SIZE);
  for (let i = 0; i < FRAME_SIZE; i++) data[i] = (i * 37 + 11) % 256;
  const digest = crypto.createHash('sha256').update(toNativeFormat(data)).digest('hex');
  assert.strictEqual(digest, 'c94eac32b46614437efa4e68d4493e213ab0d0a04e6b19ae532fae0cd92f64d3');
}

// -- JSON commands --------------------------------------------------------

function countingDisplay() {
  return {
    cleared: 0,
    shown: 0,
    async clear() { this.cleared++; },
    async showBitmap() { this.shown++; },
  };
}

(async () => {
  const display = countingDisplay();

  assert.deepStrictEqual(await handleCommand(display, '{"id": 7, "clear": true}'), {
    id: 7,
    ok: true,
  });
  assert.strictEqual(display.cleared, 1);

  const image = Buffer.alloc(FRAME_SIZE).toString('base64');
  assert.deepStrictEqual(await handleCommand(display, `{"id": 8, "image": "${image}"}`), {
    id: 8,
    ok: true,
  });
  assert.strictEqual(display.shown, 1);

  const short = Buffer.alloc(10).toString('base64');
  const nacks = [
    ['not json', null],
    ['{"clear": true}', null], // no id
    ['{"id": "7", "clear": true}', null], // id is not an integer
    ['{"id": 9}', 9], // no action
    ['{"id": 10, "image": "not base64!!"}', 10], // undecodable
    [`{"id": 11, "image": "${short}"}`, 11], // wrong size
  ];
  for (const [message, wantId] of nacks) {
    const reply = await handleCommand(display, message);
    assert.strictEqual(reply.ok, false, `${message} must be rejected`);
    assert.strictEqual(reply.id, wantId, `${message} id = ${reply.id}`);
    assert.ok(reply.error, `${message} must carry an error`);
  }
  // None of them reached the panel.
  assert.strictEqual(display.cleared, 1);
  assert.strictEqual(display.shown, 1);

  // Service managers stop nodes with SIGTERM. It must take the same display
  // clearing path as an interactive Ctrl-C (SIGINT).
  const signalTarget = new EventEmitter();
  const shutdownDisplay = countingDisplay();
  let exitCode;
  let finishExit;
  const exited = new Promise((resolve) => { finishExit = resolve; });
  installShutdownHandlers(shutdownDisplay, signalTarget, (code) => {
    exitCode = code;
    finishExit();
  });
  signalTarget.emit('SIGTERM');
  await exited;
  assert.strictEqual(shutdownDisplay.cleared, 1, 'SIGTERM must clear the panel');
  assert.strictEqual(exitCode, 0, 'clean shutdown exit code');

  console.log(`OK: WebSocket errors, transposition (${WIDTH}x${HEIGHT}), command ACKs and SIGTERM cleanup`);
})();
