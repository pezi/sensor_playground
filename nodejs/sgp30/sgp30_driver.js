'use strict';
/*
 * Compact SGP30 driver, ported from the Pimoroni sgp30-python driver
 * (https://github.com/pimoroni/sgp30-python, MIT) — the library the Python
 * node uses — so the behavior matches it: big-endian 16-bit command words,
 * CRC-8-checked response words, and the blocking warm-up that discards the
 * sensor's initialization readings (fixed 400 ppm / 0 ppb).
 *
 * The SGP30 is not a register-map device — a command is a plain 2-byte
 * write and the response a plain read (i2cWrite/i2cRead, no register).
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const SGP30_ADDR = 0x58;

const CMD_INIT_AIR_QUALITY = 0x2003; // no response words
const CMD_MEASURE_AIR_QUALITY = 0x2008; // 2 response words: eCO2 ppm, TVOC ppb

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * The 8-bit CRC of a 16-bit word as defined in section 6.6 of the SGP30
 * datasheet: polynomial 0x31 (x8 + x5 + x4 + 1), initialization 0xFF, no
 * reflection, no final XOR.
 */
function calculateCrc(word) {
  let crc = 0xff;
  for (const byte of [(word >> 8) & 0xff, word & 0xff]) {
    crc ^= byte;
    for (let i = 0; i < 8; i++) {
      crc = crc & 0x80 ? ((crc << 1) ^ 0x31) & 0xff : (crc << 1) & 0xff;
    }
  }
  return crc;
}

/*
 * Verify the per-word CRC of a response buffer (each 16-bit word is
 * followed by its CRC byte) and return the words.
 */
function parseWords(buf) {
  const words = [];
  for (let i = 0; i + 2 < buf.length; i += 3) {
    const word = (buf[i] << 8) | buf[i + 1];
    if (buf[i + 2] !== calculateCrc(word)) {
      throw new Error(
        `invalid CRC in response from SGP30: ${buf[i + 2].toString(16)} != ` +
        calculateCrc(word).toString(16)
      );
    }
    words.push(word);
  }
  return words;
}

class SGP30Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x58. */
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
    return new SGP30Driver(bus);
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = SGP30_ADDR;
  }

  /*
   * Write a 16-bit command word, wait the reference driver's fixed 25 ms
   * and read responseWords CRC-checked 16-bit words back.
   */
  async _command(cmd, responseWords) {
    await this._bus.i2cWrite(this._addr, 2, Buffer.from([cmd >> 8, cmd & 0xff]));
    await sleep(25);
    if (!responseWords) return [];
    const buf = Buffer.alloc(responseWords * 3);
    await this._bus.i2cRead(this._addr, buf.length, buf);
    return parseWords(buf);
  }

  /*
   * Start air quality measurement, mirroring the reference driver's
   * start_measurement: after init_air_quality the SGP30 returns fixed
   * 400 ppm / 0 ppb readings for about 15 s (page 8/15 of the datasheet),
   * so readings are discarded until they change — capped at 20 test
   * samples to avoid a potential infinite loop.
   */
  async startMeasurement() {
    await this._command(CMD_INIT_AIR_QUALITY, 0);
    let testSamples = 0;
    for (;;) {
      const [eco2, tvoc] = await this._command(CMD_MEASURE_AIR_QUALITY, 2);
      if (eco2 !== 400 || tvoc !== 0 || testSamples >= 20) return;
      await sleep(1000);
      testSamples++;
    }
  }

  /** One air-quality measurement: { eco2 (ppm), tvoc (ppb) }. */
  async read() {
    const [eco2, tvoc] = await this._command(CMD_MEASURE_AIR_QUALITY, 2);
    return { eco2, tvoc };
  }
}

module.exports = { SGP30Driver, calculateCrc, parseWords };
