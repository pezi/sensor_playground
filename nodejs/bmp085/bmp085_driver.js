'use strict';
/*
 * BMP085 / BMP180 barometer driver — a direct port of the integer
 * compensation algorithm written out in python/bmp085/sensor_node.py (from
 * the BMP085 datasheet, section 3.5). Every intermediate value stays well
 * inside JavaScript's 32-bit bitwise range or is computed with division,
 * so plain Numbers are safe here (unlike the BME680's formulas).
 * The pin-compatible BMP180 works unchanged.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const I2C_ADDRESS = 0x77; // fixed — the BMP085 has no address pin

const REG_CALIBRATION = 0xaa; // 22 bytes: AC1..AC6, B1, B2, MB, MC, MD
const REG_CHIP_ID = 0xd0; // reads 0x55 on a BMP085 (and on a BMP180)
const REG_CONTROL = 0xf4;
const REG_DATA = 0xf6;

const CMD_READ_TEMPERATURE = 0x2e;
const CMD_READ_PRESSURE = 0x34;

const CHIP_ID = 0x55;

// Conversion time per oversampling setting, in ms (datasheet table 3,
// rounded up for margin).
const CONVERSION_TIME_MS = [5, 8, 14, 26];

// Standard sea-level pressure, in pascal.
const SEA_LEVEL_PA = 101325.0;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Parse the 22 calibration bytes. AC4, AC5 and AC6 are the only unsigned
 * words. The datasheet states no calibration word is ever 0x0000 or
 * 0xFFFF — exactly what a bus with nothing on it reads back — so that is
 * rejected here rather than compensated with.
 */
function parseCalibration(data) {
  if (data.length !== 22) {
    throw new Error(`need 22 calibration bytes, got ${data.length}`);
  }
  const signedness = [true, true, true, false, false, false, true, true, true, true, true];
  const values = signedness.map((signed, index) => {
    const raw = (data[index * 2] << 8) | data[index * 2 + 1];
    if (raw === 0x0000 || raw === 0xffff) {
      throw new Error(
        `implausible calibration word ${index} = 0x${raw.toString(16).toUpperCase()} (bad I2C read?)`
      );
    }
    return signed && raw > 0x7fff ? raw - 0x10000 : raw;
  });
  const [ac1, ac2, ac3, ac4, ac5, ac6, b1, b2, mb, mc, md] = values;
  return { ac1, ac2, ac3, ac4, ac5, ac6, b1, b2, mb, mc, md };
}

/** Integer division truncating toward zero, as C's / does. */
const truncDiv = (numerator, denominator) => Math.trunc(numerator / denominator);

/*
 * Raw readings -> { temperature °C, pressurePa } — a direct transcription
 * of the integer algorithm in the BMP085 datasheet.
 */
function compensate(c, rawTemperature, rawPressure, oversampling = 0) {
  // Temperature.
  let x1 = ((rawTemperature - c.ac6) * c.ac5) >> 15;
  let x2 = truncDiv(c.mc * 2048, x1 + c.md);
  const b5 = x1 + x2;
  const temperature = ((b5 + 8) >> 4) / 10.0; // datasheet yields 0.1 °C steps

  // Pressure.
  const b6 = b5 - 4000;
  x1 = (c.b2 * ((b6 * b6) >> 12)) >> 11;
  x2 = (c.ac2 * b6) >> 11;
  let x3 = x1 + x2;
  const b3 = (((c.ac1 * 4 + x3) << oversampling) + 2) >> 2;
  x1 = (c.ac3 * b6) >> 13;
  x2 = (c.b1 * ((b6 * b6) >> 12)) >> 16;
  x3 = (x1 + x2 + 2) >> 2;
  const b4 = (c.ac4 * (x3 + 32768)) >> 15;
  const b7 = (rawPressure - b3) * (50000 >> oversampling);
  // B7 is unsigned 32-bit in the datasheet and can exceed 2^31 (and JS's
  // bitwise range), which is why it is only ever divided, never shifted.
  let pressure = b7 < 0x80000000 ? truncDiv(b7 * 2, b4) : truncDiv(b7, b4) * 2;
  x1 = (pressure >> 8) * (pressure >> 8);
  x1 = (x1 * 3038) >> 16;
  x2 = (-7357 * pressure) >> 16;
  pressure += (x1 + x2 + 3791) >> 4;

  return { temperature, pressurePa: pressure };
}

/*
 * Altitude in metres from pressure, per the international barometric
 * formula the BMP085 datasheet quotes.
 */
function altitudeFor(pressurePa, seaLevelPa = SEA_LEVEL_PA) {
  return 44330.0 * (1.0 - (pressurePa / seaLevelPa) ** (1.0 / 5.255));
}

class BMP085Driver {
  /** Open the sensor on /dev/i2c-<bus>, verify the chip id, load calibration. */
  static async create(busNumber, oversampling = 3) {
    if (!(oversampling in CONVERSION_TIME_MS)) {
      throw new Error(`oversampling must be 0-3, got ${oversampling}`);
    }
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
    const driver = new BMP085Driver(bus, oversampling);
    await driver._init();
    return driver;
  }

  constructor(bus, oversampling) {
    this._bus = bus;
    this._oversampling = oversampling;
    this._cal = null;
  }

  async _readRegs(reg, length) {
    const buf = Buffer.alloc(length);
    await this._bus.readI2cBlock(I2C_ADDRESS, reg, length, buf);
    return buf;
  }

  async _init() {
    const chipId = await this._bus.readByte(I2C_ADDRESS, REG_CHIP_ID);
    if (chipId !== CHIP_ID) {
      throw new Error(
        `No BMP085 at address 0x${I2C_ADDRESS.toString(16)}: chip id is ` +
        `0x${chipId.toString(16)}, expected 0x${CHIP_ID.toString(16)}.`
      );
    }
    this._cal = parseCalibration(await this._readRegs(REG_CALIBRATION, 22));
  }

  async _readRawTemperature() {
    await this._bus.writeByte(I2C_ADDRESS, REG_CONTROL, CMD_READ_TEMPERATURE);
    await sleep(CONVERSION_TIME_MS[0]); // temperature ignores oversampling
    const data = await this._readRegs(REG_DATA, 2);
    return (data[0] << 8) | data[1];
  }

  async _readRawPressure() {
    await this._bus.writeByte(I2C_ADDRESS, REG_CONTROL,
      CMD_READ_PRESSURE + (this._oversampling << 6));
    await sleep(CONVERSION_TIME_MS[this._oversampling]);
    const data = await this._readRegs(REG_DATA, 3);
    const raw = (data[0] << 16) | (data[1] << 8) | data[2];
    return raw >> (8 - this._oversampling);
  }

  /*
   * One measurement: temperature first, and every time — its B5 term
   * feeds the pressure compensation, so a stale one skews the pressure
   * as the chip warms.
   */
  async read() {
    const rawT = await this._readRawTemperature();
    const rawP = await this._readRawPressure();
    return compensate(this._cal, rawT, rawP, this._oversampling);
  }
}

module.exports = { BMP085Driver, parseCalibration, compensate, altitudeFor, SEA_LEVEL_PA };
