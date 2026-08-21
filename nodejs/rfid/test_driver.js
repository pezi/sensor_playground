#!/usr/bin/env node
'use strict';
/*
 * Unit test: RDM630 frame parsing and XOR checksum, fed byte-by-byte the
 * way the UART delivers them.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { RdmFrameParser, checksumOk, STX, ETX } = require('./rfid_driver');

function feedAll(parser, bytes) {
  const tags = [];
  for (const byte of bytes) {
    const tag = parser.feed(byte);
    if (tag !== null) tags.push(tag);
  }
  return tags;
}

/** Wrap 12 hex chars in STX/ETX. */
const frame = (chars) => [STX, ...Buffer.from(chars, 'ascii'), ETX];

// A valid frame yields its tag.
assert.deepStrictEqual(feedAll(new RdmFrameParser(), frame('0F0024ADAB2D')), ['0F0024ADAB']);

// Lowercase hex is accepted and the tag uppercased.
assert.deepStrictEqual(feedAll(new RdmFrameParser(), frame('0f0024adab2d')), ['0F0024ADAB']);

// A wrong checksum is rejected.
assert.deepStrictEqual(feedAll(new RdmFrameParser(), frame('0F0024ADAB2C')), []);

// A short frame is rejected.
assert.deepStrictEqual(feedAll(new RdmFrameParser(), frame('0F0024ADAB')), []);

// Overflow (more than 12 hex chars) is discarded.
assert.deepStrictEqual(feedAll(new RdmFrameParser(), frame('0F0024ADAB2D00')), []);

// A second STX mid-frame resyncs onto the new frame.
assert.deepStrictEqual(
  feedAll(new RdmFrameParser(), [STX, ...Buffer.from('0F00'), ...frame('0F0024ADAB2D')]),
  ['0F0024ADAB']
);

// Noise outside a frame is ignored (including a stray ETX)...
const parser = new RdmFrameParser();
assert.deepStrictEqual(feedAll(parser, [0x00, 0x41, ETX]), []);
// ...non-hex noise mid-frame aborts the frame, and parsing recovers on
// the next one.
assert.deepStrictEqual(
  feedAll(parser, [STX, ...Buffer.from('0F0$'), ...frame('1000C0FFEEC1')]),
  ['1000C0FFEE']
);

// Checksums of the emulation tag pool.
for (const valid of ['0F0024ADAB2D', '0A0031B2C44D', '03004F19AAFF', '1000C0FFEEC1']) {
  assert.ok(checksumOk(valid), `checksumOk(${valid})`);
}
assert.ok(!checksumOk('0F0024ADAB2C'));

console.log('OK: RDM630 frame parsing and checksum behave like the Python node');
