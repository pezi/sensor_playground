'use strict';
/*
 * Compact TSL2591 driver — a port of the `adafruit_tsl2591` library the
 * Python node uses: the same device-ID check, the same gain and
 * integration-time encoding, and the same two-equation lux formula from
 * the Adafruit Arduino library, so the values match the Python node.
 *
 * Every register access ORs the address with the command bit 0xA0 (command
 * + normal operation), exactly as the Python library does.
 *
 * The lux maths is platform-neutral (and unit tested); only the I2C access
 * requires the `i2c-bus` package (Linux only), which is loaded lazily so
 * that emulation mode works without it.
 */

// -- TSL2591 register map -------------------------------------------------

const ADDRESS = 0x29;
const COMMAND_BIT = 0xa0; // every register access is OR'd with this

const REG_ENABLE = 0x00;
const REG_CONTROL = 0x01;
const REG_DEVICE_ID = 0x12;
const REG_CHAN0_LOW = 0x14; // broadband (visible + IR), then channel 1 at 0x16

const ENABLE_POWER_ON = 0x01;
const ENABLE_AEN = 0x02; // ALS enable

const DEVICE_ID = 0x50;

// The ADC is 16-bit, but at the shortest integration time it only counts
// to 0x8FFF.
const MAX_COUNT_100MS = 0x8fff;
const MAX_COUNT = 0xffff;

// Lux equation coefficients (Adafruit Arduino library).
const LUX_DF = 408.0;
const LUX_COEF_B = 1.64;
const LUX_COEF_C = 0.59;
const LUX_COEF_D = 0.86;

// Counts at which the current gain is judged too high or too low.
const SATURATION_COUNTS = 0xffff;
const TOO_DARK_COUNTS = 100;

// Gain names accepted in config.json, in ascending order.
const GAIN_NAMES = ['low', 'med', 'high', 'max'];
const GAIN_REGISTERS = [0x00, 0x10, 0x20, 0x30];
const GAIN_FACTORS = [1.0, 25.0, 428.0, 9876.0];

// Integration time in milliseconds -> register value (0..5).
const INTEGRATION_REGISTER = { 100: 0, 200: 1, 300: 2, 400: 3, 500: 4, 600: 5 };

/*
 * Convert the two raw channel counts to lux, given the integration time
 * register value and the gain factor. Returns null when a channel
 * saturated: the counts still show the app that it is very bright, but the
 * lux value would be wrong.
 */
function calculateLux(channel0, channel1, integrationRegisterValue, gain) {
  const atime = 100.0 * integrationRegisterValue + 100.0;
  const maxCounts = integrationRegisterValue === 0 ? MAX_COUNT_100MS : MAX_COUNT;
  if (channel0 >= maxCounts || channel1 >= maxCounts) return null;
  const cpl = (atime * gain) / LUX_DF;
  // Two approximations of the visible response; the library takes
  // whichever is larger.
  const lux1 = (channel0 - LUX_COEF_B * channel1) / cpl;
  const lux2 = (LUX_COEF_C * channel0 - LUX_COEF_D * channel1) / cpl;
  return Math.max(lux1, lux2);
}

// -- Hardware -------------------------------------------------------------

class TSL2591Driver {
  /*
   * Open the sensor on /dev/i2c-<bus> at 0x29, verify the device ID and
   * program the configured gain and integration time.
   */
  static async create(busNumber, gainName, integrationMs, autoGain) {
    const gainIndex = GAIN_NAMES.indexOf(gainName);
    if (gainIndex < 0) {
      throw new Error(`gain must be one of ${GAIN_NAMES.join(', ')}, got ${gainName}`);
    }
    const integrationValue = INTEGRATION_REGISTER[integrationMs];
    if (integrationValue === undefined) {
      throw new Error(
        `integration_ms must be one of ${Object.keys(INTEGRATION_REGISTER).join(', ')}, ` +
        `got ${integrationMs}`
      );
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
    const driver = new TSL2591Driver(bus, gainIndex, integrationValue, autoGain);
    const deviceId = await driver._readU8(REG_DEVICE_ID);
    if (deviceId !== DEVICE_ID) {
      throw new Error(
        `No TSL2591 at 0x29 (device ID 0x${deviceId.toString(16)}, ` +
        `expected 0x${DEVICE_ID.toString(16)})`
      );
    }
    await driver._applyGain();
    await driver._applyIntegration();
    // Power on and enable the ALS; the chip then integrates continuously.
    await driver._writeU8(REG_ENABLE, ENABLE_POWER_ON | ENABLE_AEN);
    return driver;
  }

  constructor(bus, gainIndex, integrationValue, autoGain) {
    this._bus = bus;
    this._gainIndex = gainIndex;
    this._integrationValue = integrationValue;
    this._autoGain = autoGain;
  }

  _readU8(reg) {
    return this._bus.readByte(ADDRESS, COMMAND_BIT | reg);
  }

  _writeU8(reg, value) {
    return this._bus.writeByte(ADDRESS, COMMAND_BIT | reg, value);
  }

  /* Write the gain bits, leaving the integration bits alone. */
  async _applyGain() {
    const control = await this._readU8(REG_CONTROL);
    await this._writeU8(REG_CONTROL, (control & 0b11001111) | GAIN_REGISTERS[this._gainIndex]);
  }

  /* Write the integration bits, leaving the gain alone. */
  async _applyIntegration() {
    const control = await this._readU8(REG_CONTROL);
    await this._writeU8(REG_CONTROL, (control & 0b11111000) | this._integrationValue);
  }

  /* The broadband (visible + IR) and infrared counts. */
  async rawLuminosity() {
    const buf = Buffer.alloc(4);
    await this._bus.readI2cBlock(ADDRESS, COMMAND_BIT | REG_CHAN0_LOW, 4, buf);
    return { broadband: buf[0] | (buf[1] << 8), infrared: buf[2] | (buf[3] << 8) };
  }

  /* Lux for the two counts at the currently programmed gain/integration. */
  lux(broadband, infrared) {
    return calculateLux(broadband, infrared, this._integrationValue, GAIN_FACTORS[this._gainIndex]);
  }

  /*
   * Move one gain step when the broadband channel pins at either end.
   *
   * Only one step per reading: changing the gain invalidates the
   * integration already in flight, so the *next* reading is the one that
   * benefits. Jumping straight to the extreme instead would make the value
   * oscillate whenever the light sits near a threshold.
   */
  async autoGainStep(broadband) {
    if (!this._autoGain) return;
    let index = this._gainIndex;
    if (broadband >= SATURATION_COUNTS) index = Math.max(0, index - 1);
    else if (broadband <= TOO_DARK_COUNTS) index = Math.min(GAIN_FACTORS.length - 1, index + 1);
    if (index !== this._gainIndex) {
      this._gainIndex = index;
      await this._applyGain();
    }
  }
}

module.exports = { TSL2591Driver, calculateLux, GAIN_NAMES };
