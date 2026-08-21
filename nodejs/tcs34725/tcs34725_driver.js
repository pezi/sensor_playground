'use strict';
/*
 * Compact TCS34725 driver — a port of the `adafruit_tcs34725` library the
 * Python node uses: the same sensor-ID check, the same 154 ms / 4x
 * profile, the same gamma-corrected RGB bytes, and the same DN40 lux and
 * colour temperature algorithm, so the values match the Python node.
 *
 * Every register access ORs the address with the command bit 0x80, exactly
 * as the Python library does — that is the byte sequence the reference
 * node puts on the wire, block reads included.
 *
 * The colour maths is platform-neutral (and unit tested); only the I2C
 * access requires the `i2c-bus` package (Linux only), which is loaded
 * lazily so that emulation mode works without it.
 */

// -- TCS34725 register map -----------------------------------------------

const ADDRESS = 0x29;
const COMMAND_BIT = 0x80; // every register access is OR'd with this

const REG_ENABLE = 0x00;
const REG_ATIME = 0x01;
const REG_CONTROL = 0x0f;
const REG_SENSOR_ID = 0x12;
const REG_STATUS = 0x13;
const REG_CDATA = 0x14; // C, R, G, B — eight consecutive bytes, LE words

const ENABLE_AEN = 0x02; // ADC enable
const ENABLE_PON = 0x01; // power on

const STATUS_AVALID = 0x01; // a conversion has completed

// The gain register holds the *index* into this table.
const GAINS = [1, 4, 16, 60];

// DN40 device-specific values (DN40 Table 1, Appendix I).
const GLASS_ATTENUATION = 1.0;
const DEVICE_FACTOR = 310.0;
const R_COEF = 0.136;
const G_COEF = 1.0;
const B_COEF = -0.444;
const CT_COEF = 3810.0;
const CT_OFFSET = 1391.0;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// -- Colour maths ---------------------------------------------------------

/*
 * Normalize the red, green and blue counts against the clear channel and
 * apply the library's 2.5 gamma correction, giving the 0-255 bytes the app
 * paints. Complete darkness (clear === 0) is black.
 */
function colorRGBBytes(r, g, b, clear) {
  if (clear === 0) return { red: 0, green: 0, blue: 0 };
  // Truncation at both steps, like the Python library's int() calls.
  const channel = (value) =>
    Math.min(Math.trunc(Math.pow(Math.trunc((value / clear) * 256) / 255, 2.5) * 255), 255);
  return { red: channel(r), green: channel(g), blue: channel(b) };
}

/*
 * Convert raw R/G/B/C counts to illuminance and colour temperature with the
 * algorithm from Taos/AMS design note DN40. Returns null when the clear
 * channel saturated: the sample says nothing about the colour then, so the
 * node reports no reading at all.
 */
function temperatureAndLuxDN40(r, g, b, c, integrationMs, gain) {
  // Analog/digital saturation (DN40 3.5). The ATIME register holds
  // 256 - cycles, so cycles is what the count limit scales with.
  const cycles = Math.round(integrationMs / 2.4);
  let saturation = cycles > 63 ? 65535 : 1024 * cycles;
  // Ripple saturation (DN40 3.7): below 150 ms the 50/60 Hz ripple of
  // mains-powered light eats into the usable range.
  if (integrationMs < 150) saturation -= saturation / 4;
  if (c >= saturation) return null;

  // IR rejection (DN40 3.1): the excess of R+G+B over the clear channel is
  // infrared leaking into all three colour channels.
  const infrared = r + g + b > c ? (r + g + b - c) / 2 : 0.0;
  const r2 = r - infrared;
  const g2 = g - infrared;
  const b2 = b - infrared;

  // Lux (DN40 3.2).
  const g1 = R_COEF * r2 + G_COEF * g2 + B_COEF * b2;
  let cpl = (integrationMs * gain) / (GLASS_ATTENUATION * DEVICE_FACTOR);
  if (cpl === 0) cpl = 0.001;

  // Colour temperature (DN40 3.4).
  const red2 = r2 === 0 ? 0.001 : r2;
  return { lux: g1 / cpl, colorTemperature: (CT_COEF * b2) / red2 + CT_OFFSET };
}

// -- Hardware -------------------------------------------------------------

class TCS34725Driver {
  /*
   * Open the sensor on /dev/i2c-<bus> at 0x29, verify the sensor ID and
   * program the same profile as the Python node: 154 ms integration at 4x
   * gain, with the ADC left enabled.
   *
   * The library's own defaults (2.4 ms, 1x) collect almost no light — every
   * channel reads 0 in normal room light and the readings collapse to
   * constants — which is why both nodes override them.
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
    const driver = new TCS34725Driver(bus);
    const sensorId = await driver._readU8(REG_SENSOR_ID);
    // The library accepts all three IDs the TCS3472 family reports.
    if (![0x44, 0x10, 0x4d].includes(sensorId)) {
      throw new Error(
        `No TCS34725 at 0x29 (sensor ID 0x${sensorId.toString(16)}, expected 0x44, 0x10 or 0x4d)`
      );
    }
    await driver._setIntegrationTime(154);
    await driver._setGain(4);
    await driver._activate();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
    this.integrationMs = 0;
    this.gain = 0;
  }

  _readU8(reg) {
    return this._bus.readByte(ADDRESS, COMMAND_BIT | reg);
  }

  _writeU8(reg, value) {
    return this._bus.writeByte(ADDRESS, COMMAND_BIT | reg, value);
  }

  /*
   * Program the integration time in milliseconds. The chip counts 2.4 ms
   * cycles and ATIME holds 256 - cycles.
   */
  async _setIntegrationTime(milliseconds) {
    const cycles = Math.trunc(milliseconds / 2.4);
    this.integrationMs = cycles * 2.4;
    await this._writeU8(REG_ATIME, 256 - cycles);
  }

  /* Program the analog gain; the register holds the index into GAINS. */
  async _setGain(gain) {
    const index = GAINS.indexOf(gain);
    if (index < 0) throw new Error(`gain must be one of ${GAINS.join(', ')}, got ${gain}`);
    this.gain = gain;
    await this._writeU8(REG_CONTROL, index);
  }

  /*
   * Power the chip and enable the ADC, leaving it running — the Python node
   * does the same instead of toggling it around every read.
   */
  async _activate() {
    const enable = await this._readU8(REG_ENABLE);
    await this._writeU8(REG_ENABLE, enable | ENABLE_PON);
    await sleep(3); // the oscillator needs 2.4 ms to settle
    await this._writeU8(REG_ENABLE, enable | ENABLE_PON | ENABLE_AEN);
  }

  /*
   * Wait for a completed conversion and return the raw 16-bit red, green,
   * blue and clear counts.
   */
  async readRaw() {
    // One integration period is the longest this can take; give it a few so
    // a conversion that started just before the call still counts.
    const deadline = Date.now() + 3 * this.integrationMs + 50;
    for (;;) {
      const status = await this._readU8(REG_STATUS);
      if (status & STATUS_AVALID) break;
      if (Date.now() > deadline) {
        throw new Error(`no completed conversion after ${3 * this.integrationMs + 50} ms`);
      }
      await sleep(this.integrationMs + 0.9);
    }
    const buf = Buffer.alloc(8);
    await this._bus.readI2cBlock(ADDRESS, COMMAND_BIT | REG_CDATA, 8, buf);
    const word = (lo) => buf[lo] | (buf[lo + 1] << 8);
    // The block starts at the clear channel, then red, green, blue.
    return { r: word(2), g: word(4), b: word(6), c: word(0) };
  }

  /*
   * One measurement as the REST payload, or null when the clear channel
   * saturated (no valid colour to report).
   *
   * Unlike the Python node — which reads the sensor once per property and
   * so three times per request — this derives the colour, the illuminance
   * and the colour temperature from a single conversion, which also keeps
   * the three values consistent with each other.
   */
  async read() {
    const { r, g, b, c } = await this.readRaw();
    const derived = temperatureAndLuxDN40(r, g, b, c, this.integrationMs, this.gain);
    if (derived === null) return null;
    const { red, green, blue } = colorRGBBytes(r, g, b, c);
    return {
      colorTemperature: Math.round(derived.colorTemperature),
      lux: Math.round(derived.lux),
      red,
      green,
      blue,
    };
  }
}

module.exports = { TCS34725Driver, colorRGBBytes, temperatureAndLuxDN40 };
