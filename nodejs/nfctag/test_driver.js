#!/usr/bin/env node
'use strict';
/*
 * NDEF-area parsing tests — the same vectors as the Go and Rust ports,
 * mirroring the Python node's behavior (Type 5 capability container +
 * TLV stream, Text/URI records, hex-dump fallback).
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { parseNdefArea, SCAN_LENGTH, DATA_HEX_CAP } = require('./nfctag_driver');

// Type 5 capability container (magic 0xE1).
const CC = [0xe1, 0x40, 0x40, 0x00];

const area = (...tlvs) => Buffer.from([...CC, ...tlvs.flat()]);

// NDEF short record: well-known Text, "en", "Hello".
const TEXT_HELLO = [0xd1, 0x01, 0x08, 0x54, 0x02, 0x65, 0x6e, 0x48, 0x65, 0x6c, 0x6c, 0x6f];

const cases = [
  ['blank zeros', Buffer.alloc(SCAN_LENGTH), { kind: 'empty' }],
  ['blank ff', Buffer.alloc(SCAN_LENGTH, 0xff), { kind: 'empty' }],
  ['empty scan', Buffer.alloc(0), { kind: 'empty' }],
  ['no capability container', Buffer.from([0xde, 0xad, 0xbe, 0xef]), { kind: 'data', value: 'DEADBEEF' }],
  ['short garbage', Buffer.from([0x12]), { kind: 'data', value: '12' }],
  ['text record', area([0x03, 0x0c], TEXT_HELLO, [0xfe]), { kind: 'text', value: 'Hello' }],
  [
    'uri record',
    area([0x03, 0x0d, 0xd1, 0x01, 0x09, 0x55, 0x04], [...Buffer.from('seeed.cc')], [0xfe]),
    { kind: 'uri', value: 'https://seeed.cc' },
  ],
  [
    'utf16 text record',
    area([0x03, 0x0b, 0xd1, 0x01, 0x07, 0x54, 0x82, 0x65, 0x6e, 0x48, 0x00, 0x69, 0x00], [0xfe]),
    { kind: 'text', value: 'Hi' },
  ],
  ['three-byte tlv length', area([0x03, 0xff, 0x00, 0x0c], TEXT_HELLO, [0xfe]), { kind: 'text', value: 'Hello' }],
  [
    'padding and unknown tlv skipped',
    area([0x00, 0x01, 0x02, 0xaa, 0xbb, 0x03, 0x0c], TEXT_HELLO, [0xfe]),
    { kind: 'text', value: 'Hello' },
  ],
  [
    'unknown record type',
    area([0x03, 0x06, 0xd2, 0x01, 0x02, 0x78, 0xde, 0xad, 0xfe]),
    { kind: 'data', value: 'DEAD' },
  ],
  ['terminator only', area([0xfe]), { kind: 'empty' }],
  ['zero-length ndef tlv', area([0x03, 0x00, 0xfe]), { kind: 'empty' }],
  ['message truncated by scan window', area([0x03, 0x10, 0xd1, 0x01]), { kind: 'data', value: 'D101' }],
  ['text record without payload', area([0x03, 0x04, 0xd1, 0x01, 0x00, 0x54, 0xfe]), { kind: 'data', value: 'D1010054' }],
  [
    'hex dump is capped',
    Buffer.alloc(80, 0xab),
    { kind: 'data', value: 'AB'.repeat(DATA_HEX_CAP) },
  ],
];

let n = 0;
for (const [name, data, want] of cases) {
  const got = parseNdefArea(data);
  assert.deepStrictEqual(got, want, `${name}: got ${JSON.stringify(got)}, want ${JSON.stringify(want)}`);
  n++;
}

console.log(`OK: ${n} NDEF parsing cases match the Python node`);
