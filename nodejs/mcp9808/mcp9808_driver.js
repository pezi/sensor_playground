'use strict';
/*
 * Compact MCP9808 driver, ported from the register access in the Python
 * node (smbus2): a 2-byte read of the ambient temperature register 0x05,
 * decoded as a 12-bit + sign two's-complement value in units of 1/16 °C
 * (the alert flag bits 15..13 are masked off) — readings match the Python
 * node.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const REG_AMBIENT_TEMP = 0x05;

/*
 * Convert the raw 16-bit ambient temperature register word to °C: lower
 * 12 bits are the magnitude in 1/16 °C, bit 12 is the sign (two's
 * complement), bits 15..13 are alert flags and ignored.
 */
function decodeTemperature(word) {
  let temperature = (word & 0x0fff) / 16.0;
  if (word & 0x1000) temperature -= 256.0;
  return temperature;
}

class MCP9808Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x18. */
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
    return new MCP9808Driver(bus);
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = 0x18;
  }

  /** One ambient temperature measurement in °C. */
  async read() {
    const buf = Buffer.alloc(2);
    await this._bus.readI2cBlock(this._addr, REG_AMBIENT_TEMP, 2, buf);
    return decodeTemperature((buf[0] << 8) | buf[1]);
  }
}

module.exports = { MCP9808Driver, decodeTemperature };
