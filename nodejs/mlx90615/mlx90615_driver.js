'use strict';
/*
 * Compact MLX90615 driver, ported from the register access in the Python
 * node (smbus2): SMBus word reads from RAM register 0x26 (ambient) and
 * 0x27 (object), temperature = raw * 0.02 K - 273.15, bit 15 set marks an
 * error — readings match the Python node.
 *
 * The MLX90615 is a strict SMBus part: the register address and the data
 * read must be one transfer with a repeated start. `i2c-bus`'s readWord()
 * goes through the kernel's SMBus ioctl and does exactly that — a plain
 * write-then-read would put a stop condition in between and the sensor
 * would abort the transfer.
 *
 * The conversion is platform-neutral (and unit tested); only the I2C
 * access requires the `i2c-bus` package (Linux only), which is loaded
 * lazily so that emulation mode works without it.
 */

const REG_AMBIENT = 0x26;
const REG_OBJECT = 0x27;

/*
 * Convert a raw RAM word to °C, or null when the sensor flags an error
 * (bit 15). The RAM value is the absolute temperature in units of 0.02 K.
 */
function decodeTemperature(raw) {
  if (raw & 0x8000) return null;
  return raw * 0.02 - 273.15;
}

class MLX90615Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x5b. */
  static async create(busNumber, address = 0x5b) {
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
    return new MLX90615Driver(bus, address);
  }

  constructor(bus, address) {
    this._bus = bus;
    this._addr = address;
  }

  /** One temperature in °C, or null when the sensor flagged an error. */
  async _readTemperature(register) {
    return decodeTemperature(await this._bus.readWord(this._addr, register));
  }

  /**
   * The sensor's own ambient temperature and the non-contact object
   * temperature, both in °C — or null when either channel flagged an
   * error.
   */
  async read() {
    const ambient = await this._readTemperature(REG_AMBIENT);
    const object = await this._readTemperature(REG_OBJECT);
    if (ambient === null || object === null) return null;
    return { ambient, object };
  }
}

module.exports = { MLX90615Driver, decodeTemperature };
