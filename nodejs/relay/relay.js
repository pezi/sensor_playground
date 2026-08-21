'use strict';

const I2C_COMMAND_CHANNEL_CONTROL = 0x10;

function validateChannelCount(count, field) {
  if (!Number.isInteger(count) || count < 1 || count > 8) {
    throw new Error(`${field} must describe 1..8 relay channels`);
  }
  return count;
}

function relayMask(states) {
  return states.reduce((mask, on, channel) => on ? mask | (1 << channel) : mask, 0);
}

/** Owns the state published to clients and serializes hardware writes. */
class RelayController {
  static async create(bank) {
    validateChannelCount(bank.count, 'relay bank');
    const relay = new RelayController(bank);
    try {
      await bank.write(relay.states);
    } catch (err) {
      await bank.close();
      throw err;
    }
    return relay;
  }

  constructor(bank) {
    this._bank = bank;
    this.states = Array(bank.count).fill(false);
    this._pending = true;
    this._queue = Promise.resolve();
  }

  get count() { return this.states.length; }

  _mutate(change) {
    const operation = this._queue.then(async () => {
      const next = [...this.states];
      change(next);
      await this._bank.write(next);
      // Commit only after the board accepted the write.
      this.states = next;
      this._pending = true;
    });
    // A failed operation must not poison all later commands.
    this._queue = operation.catch(() => {});
    return operation;
  }

  set(channel, on) {
    if (!Number.isInteger(channel) || channel < 0 || channel >= this.count) {
      return Promise.reject(new Error(`channel ${channel} is outside board width ${this.count}`));
    }
    return this._mutate((states) => { states[channel] = on; });
  }

  toggle(channel) {
    if (!Number.isInteger(channel) || channel < 0 || channel >= this.count) {
      return Promise.reject(new Error(`channel ${channel} is outside board width ${this.count}`));
    }
    return this._mutate((states) => { states[channel] = !states[channel]; });
  }

  setAll(on) {
    return this._mutate((states) => states.fill(on));
  }

  payload() {
    return { channels: this.count, relay: [...this.states] };
  }

  takePending() {
    if (!this._pending) return null;
    this._pending = false;
    return this.payload();
  }

  async close() {
    await this._queue;
    await this._bank.close();
  }
}

class EmulatedRelayBank {
  constructor(count) { this.count = validateChannelCount(count, 'channels'); }

  async write(states) {
    const shown = states.map((on, channel) => `${channel + 1}:${on ? 'on' : 'off'}`).join(', ');
    console.log(`[emulation] relays ${shown}`);
  }

  async close() {}
}

class LibgpiodRelayBank {
  static create(gpiod, chipName, pins, activeLow) {
    const chip = new gpiod.Chip(chipName);
    const lines = [];
    try {
      for (const pin of pins) {
        const line = chip.getLine(pin);
        line.requestOutputMode('sensor-playground-relay');
        lines.push(line);
      }
    } catch (err) {
      for (const line of lines) line.release();
      throw err;
    }
    return new LibgpiodRelayBank(chip, lines, activeLow);
  }

  constructor(chip, lines, activeLow) {
    this._chip = chip;
    this._lines = lines;
    this._activeLow = activeLow;
    this.count = lines.length;
  }

  async write(states) {
    if (states.length !== this.count) throw new Error('relay state width changed');
    states.forEach((on, channel) => {
      const high = this._activeLow ? !on : on;
      this._lines[channel].setValue(high ? 1 : 0);
    });
  }

  async close() {
    for (const line of this._lines) line.release();
    // node-libgpiod's Chip has no documented close method.
    void this._chip;
  }
}

/* libgpiod 2.x fallback. Each `gpioset -z` child owns one line until killed. */
class GpiosetRelayBank {
  static async create(chipName, pins, activeLow) {
    const { execFile } = require('child_process');
    await new Promise((resolve, reject) => {
      execFile('gpioset', ['--version'], (err) => err ? reject(new Error(
        'gpioset not found - install the gpiod package (sudo apt install gpiod)'
      )) : resolve());
    });
    return new GpiosetRelayBank(chipName, pins, activeLow);
  }

  constructor(chipName, pins, activeLow) {
    this._chipName = chipName;
    this._pins = pins;
    this._activeLow = activeLow;
    this._holders = Array(pins.length).fill(null);
    this._queue = Promise.resolve();
    this.count = pins.length;
  }

  _replaceHolder(channel, high) {
    const { spawn } = require('child_process');
    const previous = this._holders[channel];
    return new Promise((resolve, reject) => {
      const claim = () => {
        const child = spawn('gpioset', [
          '-z', '-c', this._chipName, `${this._pins[channel]}=${high ? 1 : 0}`,
        ], { stdio: 'ignore' });
        child.once('error', reject);
        child.once('spawn', () => {
          this._holders[channel] = child;
          resolve();
        });
      };
      if (previous && previous.exitCode === null) {
        previous.once('exit', claim);
        previous.kill('SIGTERM');
      } else {
        claim();
      }
    });
  }

  write(states) {
    if (states.length !== this.count) return Promise.reject(new Error('relay state width changed'));
    const operation = this._queue.then(async () => {
      for (let channel = 0; channel < states.length; channel += 1) {
        const high = this._activeLow ? !states[channel] : states[channel];
        await this._replaceHolder(channel, high);
      }
    });
    this._queue = operation.catch(() => {});
    return operation;
  }

  async close() {
    await this._queue;
    for (const holder of this._holders) {
      if (holder && holder.exitCode === null) holder.kill('SIGTERM');
    }
  }
}

class I2cRelayBank {
  static async create(busNumber, address, count) {
    let i2c;
    try {
      i2c = require('i2c-bus');
    } catch {
      throw new Error(
        "the i2c-bus package is not installed; run 'npm install' on the target or use emulation"
      );
    }
    const bus = await i2c.openPromisified(busNumber);
    return new I2cRelayBank(bus, address, count);
  }

  constructor(bus, address, count) {
    this._bus = bus;
    this._address = address;
    this.count = validateChannelCount(count, 'channels');
  }

  async write(states) {
    if (states.length !== this.count) throw new Error('relay state width changed');
    await this._bus.writeByte(this._address, I2C_COMMAND_CHANNEL_CONTROL, relayMask(states));
  }

  async close() { await this._bus.close(); }
}

function configuredChannelCount(config) {
  const iface = config.interface || 'gpio';
  if (iface === 'i2c') return validateChannelCount(config.channels ?? 4, 'channels');
  if (!Array.isArray(config.relay_pins)) throw new Error('relay_pins must be an array');
  return validateChannelCount(config.relay_pins.length, 'relay_pins');
}

function parseI2cAddress(value) {
  const text = String(value ?? '0x11');
  if (!/^(?:0x)?[0-9a-f]+$/i.test(text)) throw new Error(`invalid i2c_address ${JSON.stringify(text)}`);
  const address = Number.parseInt(text.replace(/^0x/i, ''), 16);
  if (address < 0 || address > 0x7f) throw new Error(`i2c_address ${text} is outside 0x00..0x7f`);
  return address;
}

async function makeRelayBank(config) {
  const iface = config.interface || 'gpio';
  const count = configuredChannelCount(config);
  if (config.emulation) return new EmulatedRelayBank(count);

  if (iface === 'i2c') {
    return I2cRelayBank.create(config.i2c_bus ?? 1, parseI2cAddress(config.i2c_address), count);
  }
  if (iface !== 'gpio') {
    throw new Error(`interface ${JSON.stringify(iface)} is not supported (use "gpio" or "i2c"; Arduino-based hats require Python)`);
  }

  const chipName = String(config.gpio_chip || '/dev/gpiochip0').replace('/dev/', '');
  const activeLow = config.relay_active_low === true;
  let gpiod;
  try {
    gpiod = require('node-libgpiod');
  } catch {
    console.log('node-libgpiod unavailable, driving relays via the gpioset CLI');
    return GpiosetRelayBank.create(chipName, config.relay_pins, activeLow);
  }
  return LibgpiodRelayBank.create(gpiod, chipName, config.relay_pins, activeLow);
}

async function handleJsonCommand(relay, message) {
  let command;
  try {
    command = JSON.parse(message);
  } catch {
    console.log('Ignoring malformed command');
    return;
  }

  try {
    if (typeof command.all === 'boolean') {
      console.log(`Command: all ${command.all ? 'on' : 'off'}`);
      await relay.setAll(command.all);
      return;
    }
    if (!Number.isInteger(command.ch)) {
      console.log('Ignoring command without an integer channel');
      return;
    }
    if (command.toggle === true) {
      console.log(`Command: toggle channel ${command.ch}`);
      await relay.toggle(command.ch);
      return;
    }
    if (typeof command.on !== 'boolean') {
      console.log("Ignoring command without a boolean 'on'");
      return;
    }
    console.log(`Command: channel ${command.ch} ${command.on ? 'on' : 'off'}`);
    await relay.set(command.ch, command.on);
  } catch (err) {
    console.log(`Ignoring command: ${err.message}`);
  }
}

module.exports = {
  I2C_COMMAND_CHANNEL_CONTROL,
  relayMask,
  RelayController,
  EmulatedRelayBank,
  I2cRelayBank,
  configuredChannelCount,
  parseI2cAddress,
  makeRelayBank,
  handleJsonCommand,
};
