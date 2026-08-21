#!/usr/bin/env node
'use strict';

const assert = require('assert');
const {
  relayMask,
  RelayController,
  configuredChannelCount,
  parseI2cAddress,
  handleJsonCommand,
} = require('./relay');

class RecordingBank {
  constructor(count) {
    this.count = count;
    this.writes = [];
    this.fail = false;
  }

  async write(states) {
    if (this.fail) throw new Error('write failed');
    this.writes.push([...states]);
  }

  async close() {}
}

(async () => {
  const bank = new RecordingBank(2);
  const relay = await RelayController.create(bank);
  relay.takePending();
  await handleJsonCommand(relay, '{"ch":0,"on":true}');
  await handleJsonCommand(relay, '{"ch":1,"toggle":true}');
  assert.deepStrictEqual(relay.payload(), { channels: 2, relay: [true, true] });
  await handleJsonCommand(relay, '{"all":false}');
  assert.deepStrictEqual(relay.payload().relay, [false, false]);
  assert.strictEqual(bank.writes.length, 4, 'startup + three accepted commands');
  assert.deepStrictEqual(relay.takePending(), { channels: 2, relay: [false, false] });

  for (const command of [
    'not json', '{"ch":2,"on":true}', '{"ch":-1,"on":true}',
    '{"ch":0.5,"on":true}', '{"ch":true,"on":true}',
    '{"ch":0,"on":1}', '{"toggle":true}',
  ]) {
    const ignoredBank = new RecordingBank(2);
    const ignoredRelay = await RelayController.create(ignoredBank);
    ignoredRelay.takePending();
    await handleJsonCommand(ignoredRelay, command);
    assert.strictEqual(ignoredBank.writes.length, 1, `${command} must not write hardware`);
    assert.strictEqual(ignoredRelay.takePending(), null, `${command} must not publish`);
  }

  const failingBank = new RecordingBank(1);
  const failingRelay = await RelayController.create(failingBank);
  failingRelay.takePending();
  failingBank.fail = true;
  await assert.rejects(() => failingRelay.set(0, true), /write failed/);
  assert.deepStrictEqual(failingRelay.payload().relay, [false], 'failed write must not commit');
  assert.strictEqual(failingRelay.takePending(), null, 'failed write must not publish');

  assert.strictEqual(relayMask([true, false, true, true]), 0x0d);
  assert.strictEqual(parseI2cAddress('0x11'), 0x11);
  assert.strictEqual(parseI2cAddress('12'), 0x12);
  assert.throws(() => parseI2cAddress('0x80'), /outside/);
  assert.strictEqual(configuredChannelCount({ interface: 'gpio', relay_pins: [5, 6] }), 2);
  assert.strictEqual(configuredChannelCount({ interface: 'i2c', channels: 4 }), 4);

  console.log('OK: relay commands, validation, state commits and I2C mask');
})().catch((err) => {
  console.error(err);
  process.exit(1);
});

