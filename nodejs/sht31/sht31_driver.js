'use strict';
/*
 * Minimal SHT31 driver, ported from the Python node: single-shot
 * measurement, high repeatability, no clock stretching. The port
 * additionally verifies the Sensirion CRC-8 of both data words (the
 * Python node reads but does not check the CRC bytes).
 *
 * The SHT31 is command-based, not register-mapped: the measurement is a
 * plain write of the command bytes followed (after the conversion time)
 * by a plain 6-byte read, so the driver uses i2cWrite/i2cRead instead of
 * register access.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const ADDRESS = 0x44;

// Single-shot measurement command: high repeatability, no clock
// stretching (like the Python node).
const MEASURE_CMD = Buffer.from([0x24, 0x00]);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Sensirion CRC-8 (polynomial 0x31, init 0xFF) that protects each 16-bit
 * word of the measurement frame.
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

const convertTemperature = (raw) => -45.0 + (175.0 * raw) / 65535.0;
const convertHumidity = (raw) => (100.0 * raw) / 65535.0;

/*
 * Check both CRCs of a 6-byte measurement frame (temp msb, temp lsb, crc,
 * hum msb, hum lsb, crc) and convert the raw words with the datasheet
 * formulas -> { temperature °C, humidity %RH }.
 */
function parseFrame(frame) {
  if (frame.length !== 6) throw new Error(`short SHT31 frame: ${frame.length} bytes`);
  if (crc8(frame.subarray(0, 2)) !== frame[2]) throw new Error('SHT31 temperature CRC mismatch');
  if (crc8(frame.subarray(3, 5)) !== frame[5]) throw new Error('SHT31 humidity CRC mismatch');
  const tempRaw = (frame[0] << 8) | frame[1];
  const humRaw = (frame[3] << 8) | frame[4];
  return { temperature: convertTemperature(tempRaw), humidity: convertHumidity(humRaw) };
}

class SHT31Driver {
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
    return new SHT31Driver(bus);
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = ADDRESS;
  }

  /** One single-shot measurement. */
  async read() {
    await this._bus.i2cWrite(this._addr, MEASURE_CMD.length, MEASURE_CMD);
    await sleep(20); // the Python node's conversion wait (datasheet max 15 ms)
    const frame = Buffer.alloc(6);
    await this._bus.i2cRead(this._addr, frame.length, frame);
    return parseFrame(frame);
  }
}

module.exports = { SHT31Driver, crc8, convertTemperature, convertHumidity, parseFrame };
