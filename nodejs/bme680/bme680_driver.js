'use strict';
/*
 * Compact BME680 driver, based on the Pimoroni Python driver
 * (https://github.com/pimoroni/bme680-python, MIT) with Bosch-datasheet
 * corrections. Only what the sensor node needs:
 * forced-mode measurements of temperature, pressure, humidity and gas
 * resistance with a fixed oversampling/filter/heater profile.
 *
 * The compensation formulas use BigInt throughout: JavaScript's bitwise
 * operators truncate to 32 bits and the gas-resistance intermediate
 * products exceed Number.MAX_SAFE_INTEGER.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const CHIP_ID_ADDR = 0xd0;
const CHIP_VARIANT_ADDR = 0xf0;
const CHIP_ID = 0x61;
const VARIANT_HIGH = 0x01;

const SOFT_RESET_ADDR = 0xe0;
const SOFT_RESET_CMD = 0xb6;

const COEFF_ADDR1 = 0x89, COEFF_ADDR1_LEN = 25;
const COEFF_ADDR2 = 0xe1, COEFF_ADDR2_LEN = 16;
const ADDR_RES_HEAT_VAL = 0x00;
const ADDR_RES_HEAT_RANGE = 0x02;
const ADDR_RANGE_SW_ERR = 0x04;

const FIELD0_ADDR = 0x1d, FIELD_LENGTH = 17;
const RES_HEAT0_ADDR = 0x5a, GAS_WAIT0_ADDR = 0x64;

const CONF_OS_H_ADDR = 0x72;
const CONF_T_P_MODE_ADDR = 0x74;
const CONF_ODR_FILT_ADDR = 0x75;
const CONF_ODR_RUN_GAS_NBC_ADDR = 0x71;

const NEW_DATA_MSK = 0x80;
const GAS_RANGE_MSK = 0x0f;
const HEAT_STAB_MSK = 0x10;
const GAS_VALID_MSK = 0x20;

const OSH_MSK = 0x07, OSH_POS = 0;
const OSP_MSK = 0x1c, OSP_POS = 2;
const OST_MSK = 0xe0, OST_POS = 5;
const FILTER_MSK = 0x1c, FILTER_POS = 2;
const RUN_GAS_MSK = 0x30, RUN_GAS_POS = 4;
const MODE_MSK = 0x03, MODE_POS = 0;

const FORCED_MODE = 1;
const OS_2X = 2, OS_4X = 3, OS_8X = 4;
const FILTER_SIZE_3 = 2;
const ENABLE_GAS_MEAS_LOW = 0x01, ENABLE_GAS_MEAS_HIGH = 0x02;

const POLL_PERIOD_MS = 10;

const LOOKUP_TABLE_1 = [
  2147483647n, 2147483647n, 2147483647n, 2147483647n,
  2147483647n, 2126008810n, 2147483647n, 2130303777n, 2147483647n,
  2147483647n, 2143188679n, 2136746228n, 2147483647n, 2126008810n,
  2147483647n, 2147483647n,
];
const LOOKUP_TABLE_2 = [
  4096000000n, 2048000000n, 1024000000n, 512000000n,
  255744255n, 127110228n, 64000000n, 32258064n,
  16016016n, 8000000n, 4000000n, 2000000n,
  1000000n, 500000n, 250000n, 125000n,
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// Python's // (floor division) for BigInt; BigInt / truncates toward zero.
function floorDiv(a, b) {
  const q = a / b;
  return (a % b !== 0n && ((a < 0n) !== (b < 0n))) ? q - 1n : q;
}

const twosComp8 = (v) => BigInt(v > 127 ? v - 256 : v);
const word = (msb, lsb) => BigInt((msb << 8) | lsb);
function wordSigned(msb, lsb) {
  const w = (msb << 8) | lsb;
  return BigInt(w > 32767 ? w - 65536 : w);
}

class BME680Driver {
  /**
   * Open the sensor on /dev/i2c-<bus> at 0x76 and apply the sensor node's
   * fixed profile: humidity 2x, pressure 4x, temperature 8x, IIR filter 3,
   * gas heater 320 °C for 150 ms.
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
    const driver = new BME680Driver(bus);
    await driver._init();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = 0x76;
    this._cal = {};
    this._variant = 0;
    this._tFine = 0n;
    this._ambient = 0n; // last temperature x100, for the heater calculation
    this._readQueue = Promise.resolve();
  }

  async _readReg(reg) {
    return this._bus.readByte(this._addr, reg);
  }

  async _readRegs(reg, length) {
    const buf = Buffer.alloc(length);
    // SMBus block reads cap at 32 bytes; the longest block here is 25.
    await this._bus.readI2cBlock(this._addr, reg, length, buf);
    return buf;
  }

  async _writeReg(reg, value) {
    await this._bus.writeByte(this._addr, reg, value);
  }

  async _setBits(reg, mask, pos, value) {
    let cur = await this._readReg(reg);
    cur = (cur & ~mask) | (value << pos);
    await this._writeReg(reg, cur & 0xff);
  }

  async _init() {
    const id = await this._readReg(CHIP_ID_ADDR);
    if (id !== CHIP_ID) {
      throw new Error(`BME680 not found, invalid chip id 0x${id.toString(16)}`);
    }
    this._variant = await this._readReg(CHIP_VARIANT_ADDR);

    await this._writeReg(SOFT_RESET_ADDR, SOFT_RESET_CMD);
    await sleep(10);

    await this._readCalibration();

    await this._setBits(CONF_OS_H_ADDR, OSH_MSK, OSH_POS, OS_2X);
    await this._setBits(CONF_T_P_MODE_ADDR, OSP_MSK, OSP_POS, OS_4X);
    await this._setBits(CONF_T_P_MODE_ADDR, OST_MSK, OST_POS, OS_8X);
    await this._setBits(CONF_ODR_FILT_ADDR, FILTER_MSK, FILTER_POS, FILTER_SIZE_3);
    const runGas = this._variant === VARIANT_HIGH ? ENABLE_GAS_MEAS_HIGH : ENABLE_GAS_MEAS_LOW;
    await this._setBits(CONF_ODR_RUN_GAS_NBC_ADDR, RUN_GAS_MSK, RUN_GAS_POS, runGas);

    // One initial measurement to seed the ambient temperature used by the
    // heater-resistance formula (the Pimoroni constructor does the same).
    if (await this.read() === null) {
      throw new Error('initial BME680 measurement timed out');
    }

    await this._writeReg(RES_HEAT0_ADDR, this._calcHeaterResistance(320));
    await this._writeReg(GAS_WAIT0_ADDR, calcHeaterDuration(150));
  }

  async _readCalibration() {
    const c1 = await this._readRegs(COEFF_ADDR1, COEFF_ADDR1_LEN);
    const c2 = await this._readRegs(COEFF_ADDR2, COEFF_ADDR2_LEN);
    const cal = Buffer.concat([c1, c2]);

    const heatRange = await this._readReg(ADDR_RES_HEAT_RANGE);
    const heatVal = await this._readReg(ADDR_RES_HEAT_VAL);
    const swErr = await this._readReg(ADDR_RANGE_SW_ERR);
    this._setCalibration(cal, heatRange, heatVal, swErr);
  }

  // Decode the raw calibration blocks; array indices as in the Pimoroni
  // driver's set_from_array(). Separate from the I2C read so the math is
  // unit-testable.
  _setCalibration(cal, heatRange, heatVal, swErr) {
    const c = this._cal;
    c.parT1 = word(cal[34], cal[33]);
    c.parT2 = wordSigned(cal[2], cal[1]);
    c.parT3 = twosComp8(cal[3]);
    c.parP1 = word(cal[6], cal[5]);
    c.parP2 = wordSigned(cal[8], cal[7]);
    c.parP3 = twosComp8(cal[9]);
    c.parP4 = wordSigned(cal[12], cal[11]);
    c.parP5 = wordSigned(cal[14], cal[13]);
    c.parP6 = twosComp8(cal[16]);
    c.parP7 = twosComp8(cal[15]);
    c.parP8 = wordSigned(cal[20], cal[19]);
    c.parP9 = wordSigned(cal[22], cal[21]);
    c.parP10 = BigInt(cal[23]);
    c.parH1 = BigInt((cal[27] << 4) | (cal[26] & 0x0f));
    c.parH2 = BigInt((cal[25] << 4) | (cal[26] >> 4));
    c.parH3 = twosComp8(cal[28]);
    c.parH4 = twosComp8(cal[29]);
    c.parH5 = twosComp8(cal[30]);
    c.parH6 = BigInt(cal[31]);
    c.parH7 = twosComp8(cal[32]);
    c.parGH1 = twosComp8(cal[37]);
    c.parGH2 = wordSigned(cal[36], cal[35]);
    c.parGH3 = twosComp8(cal[38]);
    c.resHeatRange = BigInt((heatRange & 0x30) >> 4);
    c.resHeatVal = twosComp8(heatVal);
    // Register 0x04<7:4> is a signed four-bit value. Sign-extend it before
    // scaling; masking a signed byte first incorrectly turns -4 into 12.
    const rangeNibble = swErr & 0xf0;
    c.rangeSwErr = BigInt(rangeNibble >= 0x80 ? rangeNibble - 0x100 : rangeNibble) / 16n;
  }

  /**
   * Trigger a forced-mode measurement. Returns
   * {temperature (°C), pressure (hPa), humidity (%RH), gasResistance (Ohm),
   * gasValid, heatStable} or null when no new data arrived within the
   * polling window.
   */
  read() {
    const result = this._readQueue.then(() => this._readOnce());
    this._readQueue = result.catch(() => {});
    return result;
  }

  async _readOnce() {
    // A set NEW_DATA bit can still describe the preceding forced sample.
    // Capture its measurement index and wait for the index to advance.
    const previousIndex = await this._readReg(FIELD0_ADDR + 1);
    await this._setBits(CONF_T_P_MODE_ADDR, MODE_MSK, MODE_POS, FORCED_MODE);

    // 30 x 10 ms: a fresh measurement takes ~190 ms with the 150 ms gas
    // heater, so the Pimoroni driver's 10-attempt window only ever caught
    // the previous cycle's data and failed outright on the first read.
    for (let attempt = 0; attempt < 30; attempt++) {
      const status = await this._readReg(FIELD0_ADDR);
      if ((status & NEW_DATA_MSK) === 0) {
        await sleep(POLL_PERIOD_MS);
        continue;
      }

      const regs = await this._readRegs(FIELD0_ADDR, FIELD_LENGTH);
      if (regs[1] === previousIndex) {
        await sleep(POLL_PERIOD_MS);
        continue;
      }

      const adcPres = BigInt((regs[2] << 12) | (regs[3] << 4) | (regs[4] >> 4));
      const adcTemp = BigInt((regs[5] << 12) | (regs[6] << 4) | (regs[7] >> 4));
      const adcHum = BigInt((regs[8] << 8) | regs[9]);
      const adcGasResLow = BigInt((regs[13] << 2) | (regs[14] >> 6));
      const adcGasResHigh = BigInt((regs[15] << 2) | (regs[16] >> 6));
      const gasRangeL = regs[14] & GAS_RANGE_MSK;
      const gasRangeH = regs[16] & GAS_RANGE_MSK;

      const heatStable = this._variant === VARIANT_HIGH
        ? (regs[16] & HEAT_STAB_MSK) > 0
        : (regs[14] & HEAT_STAB_MSK) > 0;
      const gasValid = this._variant === VARIANT_HIGH
        ? (regs[16] & GAS_VALID_MSK) > 0
        : (regs[14] & GAS_VALID_MSK) > 0;

      const tempX100 = this._calcTemperature(adcTemp);
      this._ambient = tempX100;

      const gasResistance = this._variant === VARIANT_HIGH
        ? calcGasResistanceHigh(adcGasResHigh, gasRangeH)
        : this._calcGasResistanceLow(adcGasResLow, gasRangeL);

      return {
        temperature: Number(tempX100) / 100.0,
        pressure: Number(this._calcPressure(adcPres)) / 100.0,
        humidity: Number(this._calcHumidity(adcHum)) / 1000.0,
        gasResistance,
        gasValid,
        heatStable,
      };
    }
    return null;
  }

  _calcTemperature(adc) {
    const c = this._cal;
    const var1 = (adc >> 3n) - (c.parT1 << 1n);
    const var2 = (var1 * c.parT2) >> 11n;
    let var3 = ((var1 >> 1n) * (var1 >> 1n)) >> 12n;
    var3 = (var3 * (c.parT3 << 4n)) >> 14n;
    this._tFine = var2 + var3;
    return ((this._tFine * 5n) + 128n) >> 8n;
  }

  _calcPressure(adc) {
    const c = this._cal;
    let var1 = (this._tFine >> 1n) - 64000n;
    let var2 = ((((var1 >> 2n) * (var1 >> 2n)) >> 11n) * c.parP6) >> 2n;
    var2 = var2 + ((var1 * c.parP5) << 1n);
    var2 = (var2 >> 2n) + (c.parP4 << 16n);
    var1 = (((((var1 >> 2n) * (var1 >> 2n)) >> 13n) * (c.parP3 << 5n)) >> 3n) +
      ((c.parP2 * var1) >> 1n);
    var1 = var1 >> 18n;
    var1 = ((32768n + var1) * c.parP1) >> 15n;

    let press = 1048576n - adc;
    press = (press - (var2 >> 12n)) * 3125n;
    if (press >= (1n << 31n)) {
      press = floorDiv(press, var1) << 1n;
    } else {
      press = floorDiv(press << 1n, var1);
    }

    var1 = (c.parP9 * (((press >> 3n) * (press >> 3n)) >> 13n)) >> 12n;
    var2 = ((press >> 2n) * c.parP8) >> 13n;
    const var3 = ((press >> 8n) * (press >> 8n) * (press >> 8n) * c.parP10) >> 17n;
    return press + ((var1 + var2 + var3 + (c.parP7 << 7n)) >> 4n);
  }

  _calcHumidity(adc) {
    const c = this._cal;
    const tempScaled = ((this._tFine * 5n) + 128n) >> 8n;
    const var1 = (adc - (c.parH1 * 16n)) -
      (floorDiv(tempScaled * c.parH3, 100n) >> 1n);
    const var2 = (c.parH2 *
      (floorDiv(tempScaled * c.parH4, 100n) +
        floorDiv((tempScaled * floorDiv(tempScaled * c.parH5, 100n)) >> 6n, 100n) +
        16384n)) >> 10n;
    const var3 = var1 * var2;
    const var4 = ((c.parH6 << 7n) + floorDiv(tempScaled * c.parH7, 100n)) >> 4n;
    const var5 = ((var3 >> 14n) * (var3 >> 14n)) >> 10n;
    const var6 = (var4 * var5) >> 1n;
    let hum = (((var3 + var6) >> 10n) * 1000n) >> 12n;
    if (hum < 0n) hum = 0n;
    if (hum > 100000n) hum = 100000n;
    return hum;
  }

  _calcGasResistanceLow(adc, gasRange) {
    const c = this._cal;
    const var1 = ((1340n + 5n * c.rangeSwErr) * LOOKUP_TABLE_1[gasRange]) >> 16n;
    const var2 = ((adc << 15n) - 16777216n) + var1;
    const var3 = (LOOKUP_TABLE_2[gasRange] * var1) >> 9n;
    let res = Number(var3 + (var2 >> 1n)) / Number(var2);
    if (res < 0) res = 2 ** 32 + res;
    return res;
  }

  _calcHeaterResistance(temperature) {
    temperature = Math.min(Math.max(temperature, 200), 400);
    const c = this._cal;
    const var1 = (Number(this._ambient) * Number(c.parGH3) / 1000.0) * 256.0;
    const var2 = Number(c.parGH1 + 784n) *
      (((Number(c.parGH2 + 154009n) * temperature * 5.0 / 100.0) + 3276800.0) / 10.0);
    const var3 = var1 + var2 / 2.0;
    const var4 = var3 / Number(c.resHeatRange + 4n);
    const var5 = 131.0 * Number(c.resHeatVal) + 65536.0;
    const heatrResX100 = (var4 / var5 - 250.0) * 34.0;
    return Math.trunc((heatrResX100 + 50.0) / 100.0) & 0xff;
  }
}

function calcGasResistanceHigh(adc, gasRange) {
  const var1 = 262144n >> BigInt(gasRange);
  const var2 = (adc - 512n) * 3n + 4096n;
  return (10000.0 * Number(var1)) / Number(var2) * 100.0;
}

function calcHeaterDuration(durationMs) {
  if (durationMs >= 0xfc0) return 0xff;
  let factor = 0;
  while (durationMs > 0x3f) {
    durationMs /= 4;
    factor += 1;
  }
  return Math.trunc(durationMs + factor * 64);
}

module.exports = { BME680Driver };
