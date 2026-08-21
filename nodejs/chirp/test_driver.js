#!/usr/bin/env node
'use strict';
/*
 * Unit tests for the portable Chirp math (the calibration mapping, the
 * big-endian register words, the signed temperature decoding and the
 * light inversion) plus the light state machine, which is driven against
 * a fake I2C bus.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const {
  ChirpDriver,
  moisturePercent,
  decodeWord,
  decodeTemperature,
  lightCounts,
} = require('./chirp_driver');

// The linear calibration mapping: cap_dry -> 0 %, cap_wet -> 100 %,
// clamped outside the calibrated span (the raw capacitance rises with
// moisture, so wet > dry).
const percentCases = [
  [290, 290, 520, 0], // dry calibration point
  [520, 290, 520, 100], // wet calibration point
  [405, 290, 520, 50], // midpoint
  [350, 290, 520, 26], // rounds to nearest percent
  [200, 290, 520, 0], // drier than dry -> clamps to 0
  [700, 290, 520, 100], // wetter than wet -> clamps to 100
  [300, 250, 600, 14], // other calibration points
  [600, 250, 600, 100],
];
for (const [cap, dry, wet, want] of percentCases) {
  const got = moisturePercent(cap, dry, wet);
  assert.strictEqual(got, want, `moisturePercent(${cap}, ${dry}, ${wet}) = ${got} != ${want}`);
}

// The chip's registers are big-endian 16-bit words.
const wordCases = [
  [0x00, 0x00, 0],
  [0x01, 0x2c, 300],
  [0x02, 0x08, 520],
  [0xff, 0x9c, 0xff9c],
  [0xff, 0xff, 65535],
];
for (const [hi, lo, want] of wordCases) {
  const got = decodeWord(hi, lo);
  assert.strictEqual(got, want, `decodeWord(${hi}, ${lo}) = ${got} != ${want}`);
}

// The temperature register is a signed 16-bit value in tenths of a
// degree (two's complement, like the Python node's `raw -= 0x10000`).
const tempCases = [
  [0x0000, 0.0],
  [0x0001, 0.1],
  [0x00d5, 21.3],
  [0x0d80, 345.6],
  [0x7fff, 3276.7], // largest positive value
  [0xffff, -0.1], // -1 -> -0.1 °C
  [0xff9c, -10.0],
  [0xfec4, -31.6],
  [0x8000, -3276.8], // most negative value
];
for (const [word, want] of tempCases) {
  const got = decodeTemperature(word);
  assert.ok(Math.abs(got - want) < 1e-9, `decodeTemperature(${word}) = ${got} != ${want}`);
}

// The chip counts a phototransistor discharge *up* in darkness, so the
// node inverts the raw value into brightness counts.
assert.strictEqual(lightCounts(0), 65535); // brightest
assert.strictEqual(lightCounts(65535), 0); // darkest
assert.strictEqual(lightCounts(20000), 45535); // mid-scale emulation value

/*
 * The light state machine against a fake bus: the first read only starts
 * a measurement (light stays null), and only a read at least three
 * seconds later harvests the value and starts the next measurement.
 */
class FakeBus {
  constructor(registers) {
    this.registers = registers;
    this.commands = [];
    this._pending = null;
  }

  async sendByte(addr, value) {
    this.commands.push(value);
    this._pending = value;
  }

  async i2cRead(addr, length, buffer) {
    const word = this.registers[this._pending] ?? 0;
    buffer[0] = (word >> 8) & 0xff;
    buffer[1] = word & 0xff;
    return { bytesRead: length, buffer };
  }
}

(async () => {
  const bus = new FakeBus({ 0x00: 405, 0x04: 20000, 0x05: 0xff9c, 0x07: 0x0126 });
  const driver = new ChirpDriver(bus, 0x20);

  const first = await driver.read();
  assert.strictEqual(first.capacitance, 405);
  assert.strictEqual(first.temperature, -10.0);
  assert.strictEqual(first.light, null, 'the first read must not carry a light value');
  assert.ok(bus.commands.includes(0x03), 'the first read must start a light measurement');

  const second = await driver.read();
  assert.strictEqual(second.light, null, 'a light measurement takes three seconds');

  driver._lightStarted -= 3000; // pretend the measurement has finished
  const third = await driver.read();
  assert.strictEqual(third.light, 45535, 'the finished measurement must be harvested');
  assert.strictEqual(
    bus.commands.filter((c) => c === 0x03).length, 2,
    'harvesting must start the next measurement'
  );

  console.log('OK: calibration, register decoding and light state machine');
})().catch((err) => {
  console.log(`FAILED: ${err.message}`);
  process.exit(1);
});
