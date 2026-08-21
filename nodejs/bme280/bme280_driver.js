'use strict';
/*
 * Compact BME280 driver, ported from the RPi.bme280 Python driver
 * (https://github.com/rm-hull/bme280, MIT) — the library the Python node
 * uses — so the readings match it: double-precision compensation formulas
 * from Appendix A (8.1) of the BME280 datasheet, forced mode with x1
 * oversampling. Unlike the BME680's integer formulas these are plain
 * floating point, so no BigInt is needed.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const CALIBRATION_1 = 0x88; // dig_T*, dig_P* (24 bytes)
const DIG_H1 = 0xa1;
const CALIBRATION_2 = 0xe1; // dig_H2..dig_H6 (7 bytes)
const CTRL_HUM = 0xf2;
const CTRL_MEAS = 0xf4;
const DATA = 0xf7; // press msb..hum lsb (8 bytes)

const OVERSAMPLING = 1; // x1, the RPi.bme280 default the Python node uses
const FORCED_MODE = 1;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Parse the calibration EEPROM blocks; mirrors RPi.bme280's
 * load_calibration_params: 16-bit words are little-endian; H4/H5 share a
 * nibble, assembled from *signed* byte reads exactly as the reference does.
 */
function parseCalibration(c1, h1, c2) {
  const u16 = (b, i) => b[i] | (b[i + 1] << 8);
  const s16 = (b, i) => {
    const w = u16(b, i);
    return w > 32767 ? w - 65536 : w;
  };
  const s8 = (v) => (v > 127 ? v - 256 : v);

  const e4 = s8(c2[3]);
  const e5 = s8(c2[4]);
  const e6 = s8(c2[5]);
  return {
    T1: u16(c1, 0), T2: s16(c1, 2), T3: s16(c1, 4),
    P1: u16(c1, 6), P2: s16(c1, 8), P3: s16(c1, 10),
    P4: s16(c1, 12), P5: s16(c1, 14), P6: s16(c1, 16),
    P7: s16(c1, 18), P8: s16(c1, 20), P9: s16(c1, 22),
    H1: h1,
    H2: s16(c2, 0),
    H3: s8(c2[2]),
    H4: (e4 << 4) | (e5 & 0x0f),
    H5: ((e5 >> 4) & 0x0f) | (e6 << 4),
    H6: s8(c2[6]),
  };
}

function tFine(cal, t) {
  const v1 = (t / 16384.0 - cal.T1 / 1024.0) * cal.T2;
  const v2 = (t / 131072.0 - cal.T1 / 8192.0) ** 2 * cal.T3;
  return v1 + v2;
}

/** Raw ADC values -> { temperature °C, pressure hPa, humidity %RH }. */
function compensate(cal, rawT, rawP, rawH) {
  const fine = tFine(cal, rawT);
  const temperature = fine / 5120.0;

  let res = fine - 76800.0;
  res = (rawH - (cal.H4 * 64.0 + (cal.H5 / 16384.0) * res)) *
    ((cal.H2 / 65536.0) * (1.0 + (cal.H6 / 67108864.0) * res * (1.0 + (cal.H3 / 67108864.0) * res)));
  res = res * (1.0 - (cal.H1 * res) / 524288.0);
  const humidity = Math.max(0.0, Math.min(res, 100.0));

  let v1 = fine / 2.0 - 64000.0;
  let v2 = (v1 * v1 * cal.P6) / 32768.0;
  v2 = v2 + v1 * cal.P5 * 2.0;
  v2 = v2 / 4.0 + cal.P4 * 65536.0;
  v1 = ((cal.P3 * v1 * v1) / 524288.0 + cal.P2 * v1) / 524288.0;
  v1 = (1.0 + v1 / 32768.0) * cal.P1;
  let pressure = 0;
  if (v1 !== 0) {
    let p = 1048576.0 - rawP;
    p = ((p - v2 / 4096.0) * 6250.0) / v1;
    v1 = (cal.P9 * p * p) / 2147483648.0;
    v2 = (p * cal.P8) / 32768.0;
    pressure = (p + (v1 + v2 + cal.P7) / 16.0) / 100.0;
  }
  return { temperature, pressure, humidity };
}

class BME280Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x76 and load its calibration. */
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
    const driver = new BME280Driver(bus);
    await driver._init();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = 0x76;
    this._cal = null;
  }

  async _readRegs(reg, length) {
    const buf = Buffer.alloc(length);
    await this._bus.readI2cBlock(this._addr, reg, length, buf);
    return buf;
  }

  async _init() {
    const c1 = await this._readRegs(CALIBRATION_1, 24);
    const h1 = await this._bus.readByte(this._addr, DIG_H1);
    const c2 = await this._readRegs(CALIBRATION_2, 7);
    this._cal = parseCalibration(c1, h1, c2);
  }

  /** One forced x1-oversampling measurement. */
  async read() {
    await this._bus.writeByte(this._addr, CTRL_HUM, OVERSAMPLING);
    await this._bus.writeByte(this._addr, CTRL_MEAS,
      (OVERSAMPLING << 5) | (OVERSAMPLING << 2) | FORCED_MODE);
    await sleep(12); // RPi.bme280's __calc_delay for x1/x1/x1, rounded up

    const block = await this._readRegs(DATA, 8);
    const rawP = ((block[0] << 16) | (block[1] << 8) | block[2]) >> 4;
    const rawT = ((block[3] << 16) | (block[4] << 8) | block[5]) >> 4;
    const rawH = (block[6] << 8) | block[7];
    return compensate(this._cal, rawT, rawP, rawH);
  }
}

module.exports = { BME280Driver, parseCalibration, compensate };
