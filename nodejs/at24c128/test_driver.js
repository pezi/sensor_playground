#!/usr/bin/env node
'use strict';
/*
 * EEPROM record encode/decode tests — the same vectors as the Go and Rust
 * ports, mirroring the Python node's behavior (magic 'S' 'P' + u16
 * big-endian length + UTF-8 text, an absent magic reading as empty).
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const {
  HEADER_SIZE,
  TEXT_MAX_BYTES,
  encodeRecord,
  recordLength,
  decodeRecord,
  EmulatedEeprom,
} = require('./at24c128_driver');

const bytes = (text) => Buffer.from(text, 'utf8');

let n = 0;
const check = (name, got, want) => {
  assert.deepStrictEqual(got, want, `${name}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  n++;
};

// -- encodeRecord ------------------------------------------------------------

const encodeCases = [
  ['ascii', 'Hi', [0x53, 0x50, 0x00, 0x02, 0x48, 0x69]],
  ['empty', '', [0x53, 0x50, 0x00, 0x00]],
  // "äöü" is 6 UTF-8 bytes, not 3 characters.
  ['multi-byte utf8', 'äöü', [0x53, 0x50, 0x00, 0x06, 0xc3, 0xa4, 0xc3, 0xb6, 0xc3, 0xbc]],
];
for (const [name, text, want] of encodeCases) {
  check(`encode ${name}`, encodeRecord(bytes(text)), Buffer.from(want));
}

// A length above 255 must land in the high byte of the header.
const longRecord = encodeRecord(Buffer.alloc(300, 0x61));
check('encode long length header', longRecord.subarray(0, 4), Buffer.from([0x53, 0x50, 0x01, 0x2c]));
check('encode long length size', longRecord.length, HEADER_SIZE + 300);

// -- recordLength ------------------------------------------------------------

const lengthCases = [
  ['text', [0x53, 0x50, 0x00, 0x05], 5],
  ['empty text', [0x53, 0x50, 0x00, 0x00], 0],
  ['max length', [0x53, 0x50, 0x02, 0x00], TEXT_MAX_BYTES],
  // A factory-fresh chip is all 0xFF and holds no record.
  ['factory fresh', [0xff, 0xff, 0xff, 0xff], -1],
  ['blank zeros', [0x00, 0x00, 0x00, 0x00], -1],
  ['wrong magic', [0x58, 0x50, 0x00, 0x02], -1],
  // An implausible length reads as no record rather than as garbage.
  ['length over the maximum', [0x53, 0x50, 0x02, 0x01], -1],
  ['short header', [0x53, 0x50, 0x00], -1],
];
for (const [name, header, want] of lengthCases) {
  check(`recordLength ${name}`, recordLength(Buffer.from(header)), want);
}

// -- decodeRecord ------------------------------------------------------------

const decodeCases = [
  ['ascii', [0x53, 0x50, 0x00, 0x02, 0x48, 0x69], 'Hi'],
  ['empty text', [0x53, 0x50, 0x00, 0x00], ''],
  ['factory fresh', new Array(16).fill(0xff), ''],
  ['wrong magic', [0x4e, 0x4f, 0x00, 0x02, 0x48, 0x69], ''],
  ['length over the maximum', [0x53, 0x50, 0xff, 0xff, 0x48, 0x69], ''],
  // Only the declared length is text; trailing bytes are ignored.
  ['trailing bytes ignored', [0x53, 0x50, 0x00, 0x02, 0x48, 0x69, 0x58, 0x59], 'Hi'],
  ['multi-byte utf8', [0x53, 0x50, 0x00, 0x05, 0xe2, 0x98, 0x95, 0xc3, 0xa4], '☕ä'],
  // Invalid bytes become U+FFFD, like Python's errors="replace".
  ['invalid utf8', [0x53, 0x50, 0x00, 0x03, 0x61, 0xff, 0x62], 'a�b'],
];
for (const [name, record, want] of decodeCases) {
  check(`decode ${name}`, decodeRecord(Buffer.from(record)), want);
}

// -- Round trip --------------------------------------------------------------

for (const text of ['', 'Hello EEPROM', 'äöü ☕', 'x'.repeat(TEXT_MAX_BYTES)]) {
  check(`round trip of ${bytes(text).length} bytes`, decodeRecord(encodeRecord(bytes(text))), text);
}

// -- Emulation ---------------------------------------------------------------

(async () => {
  const eeprom = new EmulatedEeprom();
  check('emulation initial text', await eeprom.readText(), 'Hello from the emulated EEPROM');
  await eeprom.writeText(bytes('äöü'));
  check('emulation stored text', await eeprom.readText(), 'äöü');

  console.log(`OK: ${n} EEPROM record cases match the Python node`);
})();
