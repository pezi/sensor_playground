'use strict';
/*
 * Compact SHT41 driver (single-shot mode), ported from the Python node:
 * high-precision measurement command 0xFD (~8 ms), then a 6-byte frame
 * [temp msb, temp lsb, crc, hum msb, hum lsb, crc] converted with the
 * datasheet formulas. Unlike the Python node this port also validates the
 * two Sensirion CRC-8 checksums (poly 0x31, init 0xFF).
 *
 * The SHT41 has no register map — the measurement command is a bare
 * single-byte write and the result a bare 6-byte read, so the driver uses
 * i2c-bus's raw i2cWrite/i2cRead instead of the register-based SMBus calls.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const ADDR = 0x44;
const MEASURE_HIGH = 0xfd; // high-precision single-shot measurement, ~8 ms

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
 * Validate the CRCs of a 6-byte measurement frame and return the raw
 * temperature and humidity words.
 */
function parseFrame(frame) {
  if (frame.length !== 6) {
    throw new Error(`SHT41 frame has ${frame.length} bytes, want 6`);
  }
  if (crc8(frame.subarray(0, 2)) !== frame[2] || crc8(frame.subarray(3, 5)) !== frame[5]) {
    throw new Error('SHT41 CRC mismatch');
  }
  return { tempRaw: (frame[0] << 8) | frame[1], humRaw: (frame[3] << 8) | frame[4] };
}

/*
 * Convert the raw words with the datasheet formulas the Python node uses:
 * temperature in °C, humidity in %RH clamped to 0..100.
 */
function convert(tempRaw, humRaw) {
  const temperature = -45.0 + (175.0 * tempRaw) / 65535.0;
  const humidity = Math.min(100.0, Math.max(0.0, -6.0 + (125.0 * humRaw) / 65535.0));
  return { temperature, humidity };
}

class SHT41Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x44. */
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
    return new SHT41Driver(bus);
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = ADDR;
  }

  /** One high-precision single-shot measurement. */
  async read() {
    await this._bus.i2cWrite(this._addr, 1, Buffer.from([MEASURE_HIGH]));
    await sleep(10);
    const frame = Buffer.alloc(6);
    await this._bus.i2cRead(this._addr, 6, frame);
    const { tempRaw, humRaw } = parseFrame(frame);
    return convert(tempRaw, humRaw);
  }
}

module.exports = { SHT41Driver, crc8, parseFrame, convert };
