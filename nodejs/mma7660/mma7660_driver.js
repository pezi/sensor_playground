'use strict';
/*
 * MMA7660 driver — the Grove 3-Axis Digital Accelerometer ±1.5g
 * (MMA7660FC), driven directly over I2C like the Python node's smbus2
 * access: each axis is a 6-bit two's-complement value with 21.33 counts
 * per g; bit 6 of a sample is the alert flag, meaning the register was
 * updated mid-read and must be read again.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const REG_X = 0x00;
const REG_MODE = 0x07;
const REG_SR = 0x08;

const ADDRESS = 0x4c;
const COUNTS_PER_G = 21.33;
const ALERT_BIT = 0x40;

/** One 6-bit two's-complement sample -> counts. */
function decodeAxis(raw) {
  return raw > 31 ? raw - 64 : raw;
}

/*
 * A 3-byte X/Y/Z block -> [x, y, z] in g, or null when any sample carries
 * the alert bit (updated mid-read — read again).
 */
function decodeAxes(raw) {
  for (const v of raw) {
    if (v & ALERT_BIT) return null;
  }
  return [raw[0], raw[1], raw[2]].map((v) => decodeAxis(v) / COUNTS_PER_G);
}

/*
 * Axis g values -> { roll, pitch, gforce }: roll / pitch angles in
 * degrees plus the total acceleration magnitude.
 */
function axesToOrientation(x, y, z) {
  const degrees = (rad) => (rad * 180) / Math.PI;
  return {
    roll: degrees(Math.atan2(y, z)),
    pitch: degrees(Math.atan2(-x, Math.sqrt(y * y + z * z))),
    gforce: Math.sqrt(x * x + y * y + z * z),
  };
}

class MMA7660Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x4c and switch it active. */
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
    // Standby to configure, 32 samples/s, then active mode.
    await bus.writeByte(ADDRESS, REG_MODE, 0x00);
    await bus.writeByte(ADDRESS, REG_SR, 0x02);
    await bus.writeByte(ADDRESS, REG_MODE, 0x01);
    return new MMA7660Driver(bus);
  }

  constructor(bus) {
    this._bus = bus;
  }

  /** Roll/pitch/g-force, re-reading while the alert bit is set. */
  async read() {
    for (let i = 0; i < 10; i++) {
      const buf = Buffer.alloc(3);
      await this._bus.readI2cBlock(ADDRESS, REG_X, 3, buf);
      const axes = decodeAxes(buf);
      if (axes === null) continue;
      return axesToOrientation(...axes);
    }
    throw new Error('MMA7660 kept reporting the alert bit');
  }
}

module.exports = { MMA7660Driver, decodeAxis, decodeAxes, axesToOrientation };
