'use strict';
/*
 * Compact ISL29125 driver, ported from the register access in the Python
 * node (smbus2): the device-ID check and reset, the three configuration
 * registers, and a 6-byte block read of the green/red/blue counts starting
 * at register 0x09 — readings match the Python node.
 *
 * The colour derivation below is platform-neutral (and unit tested); only
 * the I2C access requires the `i2c-bus` package (Linux only), which is
 * loaded lazily so that emulation mode works without it.
 */

// -- ISL29125 register map (see the SparkFun library / datasheet) ---------

const REG_DEVICE_ID = 0x00; // reads 0x7d; writing 0x46 resets the chip
const REG_CONFIG1 = 0x01;
const REG_CONFIG2 = 0x02;
const REG_CONFIG3 = 0x03;
const REG_GREEN_LOW = 0x09; // G L/H, R L/H, B L/H — six consecutive bytes

const DEVICE_ID = 0x7d;
const RESET_COMMAND = 0x46;

// CONFIG1: RGB sampling mode (0x05) in the 10,000 lux range (0x08), 16-bit.
const CONFIG1_RGB_10KLUX = 0x0d;
// CONFIG2: IR compensation on, maximum adjustment (the SparkFun default).
const CONFIG2_IR_ADJUST_HIGH = 0xbf;
const CONFIG3_NO_INTERRUPTS = 0x00;

// Approximate green-counts-to-lux factor for the 10K range at 16 bits.
const LUX_PER_COUNT = 10000.0 / 65535.0;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Turn the raw 16-bit channel counts into the REST payload: an approximate
 * illuminance from the green channel, whose spectral response resembles the
 * human eye, plus the colour normalized against the brightest channel so
 * the app can show it directly. In complete darkness there is no colour to
 * report, so the red/green/blue keys are absent.
 */
function deriveReading(green, red, blue) {
  const reading = { lux: Math.round(green * LUX_PER_COUNT) };
  const brightest = Math.max(red, green, blue);
  if (brightest > 0) {
    reading.red = Math.round((255 * red) / brightest);
    reading.green = Math.round((255 * green) / brightest);
    reading.blue = Math.round((255 * blue) / brightest);
  }
  return reading;
}

class ISL29125Driver {
  /** Open the sensor, verify the device ID and configure RGB sampling. */
  static async create(busNumber, address) {
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
    const deviceId = await bus.readByte(address, REG_DEVICE_ID);
    if (deviceId !== DEVICE_ID) {
      throw new Error(
        `No ISL29125 at 0x${address.toString(16)} ` +
        `(device ID 0x${deviceId.toString(16)}, expected 0x${DEVICE_ID.toString(16)})`
      );
    }
    await bus.writeByte(address, REG_DEVICE_ID, RESET_COMMAND);
    await sleep(100);
    await bus.writeByte(address, REG_CONFIG1, CONFIG1_RGB_10KLUX);
    await bus.writeByte(address, REG_CONFIG2, CONFIG2_IR_ADJUST_HIGH);
    await bus.writeByte(address, REG_CONFIG3, CONFIG3_NO_INTERRUPTS);
    // One conversion takes ~100 ms per channel; let the first RGB sampling
    // cycle complete before serving readings.
    await sleep(400);
    return new ISL29125Driver(bus, address);
  }

  constructor(bus, address) {
    this._bus = bus;
    this._addr = address;
  }

  /** The raw 16-bit green, red and blue counts. */
  async readChannels() {
    const buf = Buffer.alloc(6);
    await this._bus.readI2cBlock(this._addr, REG_GREEN_LOW, 6, buf);
    return {
      green: buf[0] | (buf[1] << 8),
      red: buf[2] | (buf[3] << 8),
      blue: buf[4] | (buf[5] << 8),
    };
  }
}

module.exports = { ISL29125Driver, deriveReading, LUX_PER_COUNT };
