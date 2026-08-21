'use strict';
/*
 * Compact SCD30 driver, ported from the scd30_i2c Python driver
 * (https://github.com/RequestForCoffee/scd30, MIT) — the library the
 * Python node uses: continuous measurement started at a 2 s interval,
 * data-ready polling, Sensirion CRC-8 (poly 0x31, init 0xFF) on every
 * 16-bit word, and the measurement floats assembled from two big-endian
 * words (MSW first) into an IEEE-754 single.
 *
 * The SCD30 has no register map — commands are bare 16-bit big-endian
 * writes (arguments carry their own CRC) and results are bare reads, via
 * i2c-bus's plain i2cWrite/i2cRead transfers.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const SCD30_ADDR = 0x61;

const CMD_START_PERIODIC = 0x0010; // arg: ambient pressure mbar, 0 = disabled
const CMD_SET_INTERVAL = 0x4600; // arg: measurement interval in seconds
const CMD_DATA_READY = 0x0202;
const CMD_READ_MEASURE = 0x0300; // 6 words: co2, temperature, humidity

const INTERVAL = 2; // seconds, like the Python node

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Sensirion CRC-8: polynomial 0x31, init 0xFF (datasheet example:
 * 0xBE 0xEF -> 0x92).
 */
function crc8(data) {
  let crc = 0xff;
  for (const b of data) {
    crc ^= b;
    for (let i = 0; i < 8; i++) {
      crc = crc & 0x80 ? ((crc << 1) ^ 0x31) & 0xff : (crc << 1) & 0xff;
    }
  }
  return crc;
}

/*
 * Validate the per-word CRCs of a response frame ([msb, lsb, crc] per
 * word) and return the big-endian 16-bit words, or null on a mismatch
 * (like the scd30_i2c driver returning None).
 */
function parseWords(frame) {
  if (frame.length === 0 || frame.length % 3 !== 0) return null;
  const words = [];
  for (let i = 0; i < frame.length; i += 3) {
    if (crc8(frame.slice(i, i + 2)) !== frame[i + 2]) return null;
    words.push((frame[i] << 8) | frame[i + 1]);
  }
  return words;
}

/*
 * Assemble two big-endian 16-bit words (MSW first) into an IEEE-754
 * single-precision float, as the scd30_i2c driver does.
 */
function decodeFloat(msw, lsw) {
  const buf = Buffer.alloc(4);
  buf.writeUInt16BE(msw, 0);
  buf.writeUInt16BE(lsw, 2);
  return buf.readFloatBE(0);
}

/*
 * Decode the 18-byte read-measurement frame into
 * { co2 ppm, temperature °C, humidity %RH }, or null on a CRC mismatch.
 */
function decodeMeasurement(frame) {
  const words = parseWords(frame);
  if (!words || words.length !== 6) return null;
  return {
    co2: decodeFloat(words[0], words[1]),
    temperature: decodeFloat(words[2], words[3]),
    humidity: decodeFloat(words[4], words[5]),
  };
}

class SCD30Driver {
  /*
   * Open the sensor on /dev/i2c-<bus> at 0x61, set the 2 s measurement
   * interval and start continuous measurement (ambient pressure
   * compensation disabled), like the Python node.
   */
  static async create(busNumber) {
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
    const driver = new SCD30Driver(bus);
    await driver._init();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = SCD30_ADDR;
  }

  /*
   * Write a big-endian command word plus CRC-protected arguments, then
   * wait the >3 ms the datasheet requires between I2C transactions (the
   * scd30_i2c driver waits 5 ms).
   */
  async _sendCommand(cmd, args = []) {
    const msg = [cmd >> 8, cmd & 0xff];
    for (const a of args) {
      const word = [a >> 8, a & 0xff];
      msg.push(word[0], word[1], crc8(word));
    }
    const buf = Buffer.from(msg);
    await this._bus.i2cWrite(this._addr, buf.length, buf);
    await sleep(5);
  }

  /** Read n CRC-protected words from the sensor, or null on a mismatch. */
  async _readWords(n) {
    const buf = Buffer.alloc(3 * n);
    await this._bus.i2cRead(this._addr, buf.length, buf);
    return parseWords(buf);
  }

  async _init() {
    // Set the interval, then read back and discard the echoed word (the
    // scd30_i2c driver does the same).
    await this._sendCommand(CMD_SET_INTERVAL, [INTERVAL]);
    if ((await this._readWords(1)) === null) {
      throw new Error('setting SCD30 measurement interval: CRC mismatch');
    }
    await this._sendCommand(CMD_START_PERIODIC, [0]);
  }

  /** Poll the data-ready status word. */
  async getDataReady() {
    await this._sendCommand(CMD_DATA_READY);
    const words = await this._readWords(1);
    return words !== null && words[0] === 1;
  }

  /*
   * One measurement: { co2, temperature, humidity } once fresh data is
   * ready, or null while none is (like the Python node's read()
   * returning None).
   */
  async read() {
    if (!(await this.getDataReady())) return null;
    await this._sendCommand(CMD_READ_MEASURE);
    const frame = Buffer.alloc(18);
    await this._bus.i2cRead(this._addr, frame.length, frame);
    return decodeMeasurement(frame);
  }
}

module.exports = { SCD30Driver, crc8, parseWords, decodeFloat, decodeMeasurement };
