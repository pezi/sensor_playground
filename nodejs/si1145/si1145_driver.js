'use strict';
/*
 * Compact SI1145 driver — a port of the `SI1145` PyPI package the Python
 * node uses (itself a port of Adafruit's Arduino library): the same reset
 * sequence, the same UV calibration coefficients, the same channel list
 * and ADC settings, and the same autonomous measurement mode, so the
 * counts this node reports match the Python node's.
 *
 * The chip computes the UV index itself from the visible/IR photodiodes
 * and reports it multiplied by 100; visible and IR are raw counts (the
 * SI1145 is not lux-calibrated) that sit at a dark baseline of roughly
 * 250-260 rather than 0.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

// -- SI1145 register map -------------------------------------------------

const ADDRESS = 0x60;

const REG_PART_ID = 0x00; // reads 0x45 on an SI1145
const REG_INT_CFG = 0x03;
const REG_IRQ_EN = 0x04;
const REG_IRQ_MODE1 = 0x05;
const REG_IRQ_MODE2 = 0x06;
const REG_HW_KEY = 0x07;
const REG_MEAS_RATE0 = 0x08;
const REG_MEAS_RATE1 = 0x09;
const REG_PS_LED21 = 0x0f;
const REG_UCOEFF0 = 0x13;
const REG_PARAM_WR = 0x17;
const REG_COMMAND = 0x18;
const REG_IRQ_STAT = 0x21;
const REG_ALS_VIS_DATA = 0x22; // 16-bit little-endian, like the two below
const REG_ALS_IR_DATA = 0x24;
const REG_UV_INDEX = 0x2c;
const REG_PARAM_RD = 0x2e;

const PART_ID = 0x45;

// Commands.
const CMD_RESET = 0x01;
const CMD_PARAM_SET = 0xa0;
const CMD_PSALS_AUTO = 0x0f;

// Parameter RAM addresses.
const PARAM_CHLIST = 0x01;
const PARAM_PSLED12SEL = 0x02;
const PARAM_PS1_ADC_MUX = 0x07;
const PARAM_PS_ADC_COUNTER = 0x0a;
const PARAM_PS_ADC_GAIN = 0x0b;
const PARAM_PS_ADC_MISC = 0x0c;
const PARAM_ALS_IR_ADC_MUX = 0x0e;
const PARAM_ALS_VIS_ADC_COUNTER = 0x10;
const PARAM_ALS_VIS_ADC_GAIN = 0x11;
const PARAM_ALS_VIS_ADC_MISC = 0x12;
const PARAM_ALS_IR_ADC_COUNTER = 0x1d;
const PARAM_ALS_IR_ADC_GAIN = 0x1e;
const PARAM_ALS_IR_ADC_MISC = 0x1f;

// Parameter values.
const CHLIST_EN_UV = 0x80;
const CHLIST_EN_ALS_IR = 0x20;
const CHLIST_EN_ALS_VIS = 0x10;
const CHLIST_EN_PS1 = 0x01;
const INT_CFG_INT_OE = 0x01;
const IRQ_EN_ALS_EVERY_SAMPLE = 0x01;
const PSLED12SEL_PS1LED1 = 0x01;
const ADC_COUNTER_511CLK = 0x70;
const ADC_MUX_SMALL_IR = 0x00;
const ADC_MUX_LARGE_IR = 0x03;
const PS_ADC_MISC_RANGE = 0x20;
const PS_ADC_MISC_PS_MODE = 0x04;
const ALS_VIS_ADC_MISC_VIS_RANGE = 0x20;
const ALS_IR_ADC_MISC_RANGE = 0x20;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Convert the chip's raw UV register value to a UV index: the SI1145
 * reports the index multiplied by 100.
 */
function uvIndex(raw) {
  return raw / 100.0;
}

class SI1145Driver {
  /**
   * Open the sensor on /dev/i2c-<bus> at 0x60, reset it and start the
   * autonomous measurement loop.
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
    const driver = new SI1145Driver(bus);
    // The Python library does not check the part ID; doing so turns a
    // missing or wrong chip into a clear error instead of nonsense counts.
    const partId = await bus.readByte(ADDRESS, REG_PART_ID);
    if (partId !== PART_ID) {
      throw new Error(
        `No SI1145 at 0x${ADDRESS.toString(16)} ` +
        `(part ID 0x${partId.toString(16)}, expected 0x${PART_ID.toString(16)})`
      );
    }
    await driver._reset();
    await driver._loadCalibration();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
  }

  _write(reg, value) {
    return this._bus.writeByte(ADDRESS, reg, value);
  }

  /*
   * Stop any running measurement, clear the interrupt state and unlock the
   * chip with the hardware key from the datasheet.
   */
  async _reset() {
    for (const [reg, value] of [
      [REG_MEAS_RATE0, 0x00],
      [REG_MEAS_RATE1, 0x00],
      [REG_IRQ_EN, 0x00],
      [REG_IRQ_MODE1, 0x00],
      [REG_IRQ_MODE2, 0x00],
      [REG_INT_CFG, 0x00],
      [REG_IRQ_STAT, 0xff],
      [REG_COMMAND, CMD_RESET],
    ]) {
      await this._write(reg, value);
    }
    await sleep(10);
    await this._write(REG_HW_KEY, 0x17);
    await sleep(10);
  }

  /*
   * Write one byte of parameter RAM, which is only reachable through the
   * command register.
   */
  async _writeParam(param, value) {
    await this._write(REG_PARAM_WR, value);
    await this._write(REG_COMMAND, param | CMD_PARAM_SET);
    await this._bus.readByte(ADDRESS, REG_PARAM_RD); // read back, as the library does
  }

  /*
   * Write the UV coefficients, enable the UV, visible, IR and proximity
   * channels, configure the ADCs for the fastest clock with 511-clock
   * measurements, and start autonomous sampling every 8 ms.
   */
  async _loadCalibration() {
    // UV index coefficients from the datasheet's default calibration.
    for (const [offset, value] of [0x29, 0x89, 0x02, 0x00].entries()) {
      await this._write(REG_UCOEFF0 + offset, value);
    }
    await this._writeParam(
      PARAM_CHLIST,
      CHLIST_EN_UV | CHLIST_EN_ALS_IR | CHLIST_EN_ALS_VIS | CHLIST_EN_PS1
    );
    // Interrupt on every ALS sample (wired out on some breakouts; the node
    // polls the data registers regardless).
    await this._write(REG_INT_CFG, INT_CFG_INT_OE);
    await this._write(REG_IRQ_EN, IRQ_EN_ALS_EVERY_SAMPLE);
    // Proximity: 20 mA on LED 1, high range, large-IR photodiode.
    await this._write(REG_PS_LED21, 0x03);
    for (const [param, value] of [
      [PARAM_PS1_ADC_MUX, ADC_MUX_LARGE_IR],
      [PARAM_PSLED12SEL, PSLED12SEL_PS1LED1],
      [PARAM_PS_ADC_GAIN, 0],
      [PARAM_PS_ADC_COUNTER, ADC_COUNTER_511CLK],
      [PARAM_PS_ADC_MISC, PS_ADC_MISC_RANGE | PS_ADC_MISC_PS_MODE],
      // Ambient light: small-IR photodiode for IR, both in high range.
      [PARAM_ALS_IR_ADC_MUX, ADC_MUX_SMALL_IR],
      [PARAM_ALS_IR_ADC_GAIN, 0],
      [PARAM_ALS_IR_ADC_COUNTER, ADC_COUNTER_511CLK],
      [PARAM_ALS_IR_ADC_MISC, ALS_IR_ADC_MISC_RANGE],
      [PARAM_ALS_VIS_ADC_GAIN, 0],
      [PARAM_ALS_VIS_ADC_COUNTER, ADC_COUNTER_511CLK],
      [PARAM_ALS_VIS_ADC_MISC, ALS_VIS_ADC_MISC_VIS_RANGE],
    ]) {
      await this._writeParam(param, value);
    }
    // 255 * 31.25 µs ≈ 8 ms between autonomous measurements.
    await this._write(REG_MEAS_RATE0, 0xff);
    await this._write(REG_COMMAND, CMD_PSALS_AUTO);
  }

  /** The latest visible and IR counts and the UV index. */
  async read() {
    const visible = await this._bus.readWord(ADDRESS, REG_ALS_VIS_DATA);
    const ir = await this._bus.readWord(ADDRESS, REG_ALS_IR_DATA);
    const rawUv = await this._bus.readWord(ADDRESS, REG_UV_INDEX);
    return { visible, ir, uv: uvIndex(rawUv) };
  }
}

module.exports = { SI1145Driver, uvIndex };
