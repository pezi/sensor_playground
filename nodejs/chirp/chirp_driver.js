'use strict';
/*
 * Compact Chirp driver, ported from the register access in the Python
 * node (smbus2) — same register map as the Arduino reference library
 * (https://github.com/Apollon77/I2CSoilMoistureSensor): a reset at
 * startup, 16-bit big-endian reads of the capacitance, temperature and
 * light registers, and the light measurement started by writing register
 * 0x03.
 *
 * The chip answers a register read as a separate transaction after a
 * short pause (the Arduino library waits 20 ms), so the driver sends the
 * register byte and reads the two data bytes itself instead of using the
 * combined SMBus block read; the reset and light commands are plain
 * one-byte writes (SMBus "send byte").
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const REG_GET_CAPACITANCE = 0x00;
const REG_MEASURE_LIGHT = 0x03;
const REG_GET_LIGHT = 0x04;
const REG_GET_TEMPERATURE = 0x05;
const REG_RESET = 0x06;
const REG_GET_VERSION = 0x07;

// A light measurement takes up to three seconds on the chip.
const LIGHT_MEASURE_MS = 3000;

// The chip needs a moment between the register write and the read; the
// Arduino reference library waits 20 ms (the Python node instead gets one
// combined SMBus transaction from smbus2).
const READ_DELAY_MS = 20;

// The chip needs a moment after a reset.
const RESET_DELAY_MS = 1000;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Maps a raw capacitance onto 0-100 % between the dry and wet calibration
 * points (the capacitance rises with moisture, so wet > dry); values
 * outside the calibrated span clamp to 0/100 %.
 */
const moisturePercent = (capacitance, capDry, capWet) => {
  const span = capWet - capDry;
  const p = (100 * (capacitance - capDry)) / span;
  return Math.round(Math.min(100, Math.max(0, p)));
};

/** Assembles the chip's big-endian 16-bit register value. */
const decodeWord = (hi, lo) => (hi << 8) | lo;

/*
 * Converts the raw temperature register word to °C: the chip reports a
 * signed 16-bit value in tenths of a degree.
 */
const decodeTemperature = (word) => (word >= 0x8000 ? word - 0x10000 : word) / 10;

/*
 * Turns the raw light register value into brightness counts: the chip
 * times a phototransistor discharge and so counts *up* in darkness,
 * which the node inverts (higher = brighter).
 */
const lightCounts = (raw) => 65535 - raw;

class ChirpDriver {
  /** Open the sensor on /dev/i2c-<bus>, reset it and report the firmware version. */
  static async create(busNumber, address) {
    let i2c;
    try {
      i2c = require('i2c-bus');
    } catch {
      console.log(
        "Error: the i2c-bus package is not installed. It is an *optional* npm\n" +
        "dependency (so a failed native build does not stop `npm install`).\n" +
        "In this node's folder run:\n" +
        "    sudo apt install -y build-essential python3\n" +
        "    npm install\n" +
        "and check the output for i2c-bus build errors. Or set\n" +
        '"emulation": true in config.json to run without hardware.'
      );
      process.exit(1);
    }
    const bus = await i2c.openPromisified(busNumber);
    const driver = new ChirpDriver(bus, address);
    await driver._init();
    return driver;
  }

  constructor(bus, address) {
    this._bus = bus;
    this._addr = address;
    this._light = null;
    this._lightStarted = null;
    // The REST and discovery paths share the sensor, so reads are
    // serialized on this promise chain (the light state machine and the
    // I2C transactions must not interleave).
    this._queue = Promise.resolve();
  }

  async _init() {
    await this._writeByte(REG_RESET);
    await sleep(RESET_DELAY_MS);
    const version = (await this._readU16(REG_GET_VERSION)) & 0xff;
    console.log(
      `Chirp sensor at 0x${this._addr.toString(16).padStart(2, '0')}, ` +
      `firmware version 0x${version.toString(16).padStart(2, '0')}`
    );
  }

  /** Send a bare command byte (SMBus "send byte"). */
  async _writeByte(value) {
    await this._bus.sendByte(this._addr, value);
  }

  /** Read a big-endian 16-bit register. */
  async _readU16(register) {
    await this._bus.sendByte(this._addr, register);
    await sleep(READ_DELAY_MS);
    const buf = Buffer.alloc(2);
    await this._bus.i2cRead(this._addr, 2, buf);
    return decodeWord(buf[0], buf[1]);
  }

  /*
   * Harvests a finished light measurement and starts the next one. The
   * measurement runs on the chip, so this never blocks; the first call
   * only starts one and leaves the value null.
   */
  async _updateLight() {
    const now = Date.now();
    if (this._lightStarted !== null && now - this._lightStarted >= LIGHT_MEASURE_MS) {
      this._light = lightCounts(await this._readU16(REG_GET_LIGHT));
      this._lightStarted = null;
    }
    if (this._lightStarted === null) {
      await this._writeByte(REG_MEASURE_LIGHT);
      this._lightStarted = now;
    }
  }

  /*
   * One set of readings: { capacitance, temperature, light }, where
   * light is null until the first measurement has completed.
   */
  async read() {
    const run = this._queue.then(async () => {
      await this._updateLight();
      const capacitance = await this._readU16(REG_GET_CAPACITANCE);
      const temperature = decodeTemperature(await this._readU16(REG_GET_TEMPERATURE));
      return { capacitance, temperature, light: this._light };
    });
    // Keep the chain alive even when this read failed.
    this._queue = run.catch(() => {});
    return run;
  }
}

module.exports = { ChirpDriver, moisturePercent, decodeWord, decodeTemperature, lightCounts };
