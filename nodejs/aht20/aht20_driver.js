'use strict';
/*
 * Compact AHT10/AHT20 driver, ported from the Python node's smbus2 code.
 *
 * Protocol (identical on both chips): trigger a measurement with
 * 0xAC 0x33 0x00, wait ~80 ms, then read 6 bytes — a status byte followed
 * by 20-bit humidity and 20-bit temperature. Only the calibrate opcode
 * differs (AHT20: 0xBE, AHT10: 0xE1), so initialization tries both.
 * The chips speak raw command writes and plain reads (no register map),
 * so the driver uses i2cWrite/i2cRead instead of register transfers.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const ADDRESS = 0x38; // fixed on both chips
const CMD_CALIBRATE_AHT20 = Buffer.from([0xbe, 0x08, 0x00]);
const CMD_CALIBRATE_AHT10 = Buffer.from([0xe1, 0x08, 0x00]);
const CMD_MEASURE = Buffer.from([0xac, 0x33, 0x00]);
const STATUS_BUSY = 0x80;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Raw 6-byte measurement (status, 20-bit humidity, 20-bit temperature)
 * -> { temperature °C, humidity %RH }, or null while the sensor is busy.
 */
function parseReading(raw) {
  if (raw[0] & STATUS_BUSY) return null;
  const humRaw = (raw[1] << 12) | (raw[2] << 4) | (raw[3] >> 4);
  const tempRaw = ((raw[3] & 0x0f) << 16) | (raw[4] << 8) | raw[5];
  return {
    temperature: (tempRaw / 1048576.0) * 200.0 - 50.0,
    humidity: (humRaw / 1048576.0) * 100.0,
  };
}

class AHT20Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x38 and calibrate it. */
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
    const driver = new AHT20Driver(bus);
    await sleep(40);
    await driver._calibrate();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = ADDRESS;
    this._queue = Promise.resolve();
  }

  /** Send the calibrate command; tolerate either chip variant. */
  async _calibrate() {
    for (const command of [CMD_CALIBRATE_AHT20, CMD_CALIBRATE_AHT10]) {
      try {
        await this._bus.i2cWrite(this._addr, command.length, command);
        await sleep(10);
        return;
      } catch {
        continue;
      }
    }
  }

  /** One triggered measurement, or null while the sensor is busy. */
  async read() {
    // Discovery and REST callbacks may overlap while this method is waiting
    // for conversion. Queue the complete trigger/wait/read transaction.
    const result = this._queue.then(() => this._readOnce());
    this._queue = result.catch(() => {});
    return result;
  }

  async _readOnce() {
    await this._bus.i2cWrite(this._addr, CMD_MEASURE.length, CMD_MEASURE);
    await sleep(80);
    const { buffer } = await this._bus.i2cRead(this._addr, 6, Buffer.alloc(6));
    return parseReading(buffer);
  }
}

module.exports = { AHT20Driver, parseReading };
