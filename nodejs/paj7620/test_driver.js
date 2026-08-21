#!/usr/bin/env node
'use strict';
/*
 * Tests for the platform-neutral driver logic: the gesture-code-to-name
 * mapping (matching the Python node's GESTURES dict / the app's Gesture
 * enum) and grove.py's two-phase flag decoding, with reads and sleeps
 * injected.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const {
  GESTURES,
  decodeGesture,
  GES_ENTRY_TIME_MS,
  GES_QUIT_TIME_MS,
} = require('./paj7620_driver');

// Gesture-code-to-name mapping, exactly the Python node's GESTURES dict.
assert.deepStrictEqual(GESTURES, {
  1: 'forward',
  2: 'backward',
  3: 'right',
  4: 'left',
  5: 'up',
  6: 'down',
  7: 'clockwise',
  8: 'antiClockwise',
  9: 'wave',
});
assert.strictEqual(GESTURES[0], undefined); // 0 means "no gesture"

/** Feeds decodeGesture a scripted sequence of [register, value] reads. */
function fakeReads(reads) {
  let i = 0;
  return async (reg) => {
    assert.ok(i < reads.length, `unexpected read of register 0x${reg.toString(16)}`);
    const [expectedReg, value] = reads[i++];
    assert.strictEqual(reg, expectedReg, `read register 0x${reg.toString(16)}, want 0x${expectedReg.toString(16)}`);
    return value;
  };
}

const noSleep = async () => {};

async function run() {
  const cases = [
    ['right', [[0x43, 0x01], [0x43, 0x00]], 3],
    ['left', [[0x43, 0x02], [0x43, 0x00]], 4],
    ['up', [[0x43, 0x04], [0x43, 0x00]], 5],
    ['down', [[0x43, 0x08], [0x43, 0x00]], 6],
    ['forward', [[0x43, 0x10]], 1],
    ['backward', [[0x43, 0x20]], 2],
    ['forward after right', [[0x43, 0x01], [0x43, 0x10]], 1],
    ['backward after up', [[0x43, 0x04], [0x43, 0x20]], 2],
    ['clockwise', [[0x43, 0x40]], 7],
    ['anticlockwise', [[0x43, 0x80]], 8],
    ['wave', [[0x43, 0x00], [0x44, 0x01]], 9],
    ['nothing', [[0x43, 0x00], [0x44, 0x00]], 0],
  ];
  for (const [name, reads, code] of cases) {
    const got = await decodeGesture(fakeReads(reads), noSleep);
    assert.strictEqual(got, code, `${name}: code ${got}, want ${code}`);
  }

  // A combined gesture waits GES_ENTRY_TIME before the re-read and
  // GES_QUIT_TIME afterwards, exactly like grove.py.
  const slept = [];
  const code = await decodeGesture(
    fakeReads([[0x43, 0x01], [0x43, 0x10]]),
    async (ms) => slept.push(ms)
  );
  assert.strictEqual(code, 1);
  assert.deepStrictEqual(slept, [GES_ENTRY_TIME_MS, GES_QUIT_TIME_MS]);

  console.log(`OK: gesture map and ${cases.length} decode cases match the grove.py reference driver`);
}

run().catch((err) => {
  console.error(err);
  process.exit(1);
});
