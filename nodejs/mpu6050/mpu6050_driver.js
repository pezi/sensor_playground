'use strict';
/*
 * MPU6050 driver — the InvenSense 6-axis IMU, driven directly over I2C
 * like the Python node's smbus2 access: the chip wakes from sleep by
 * clearing PWR_MGMT_1, then a 14-byte burst read from ACCEL_XOUT_H
 * delivers the accelerometer (big-endian 16-bit words, 16384 LSB/g at the
 * default ±2 g range), temperature (raw/340 + 36.53 °C) and gyroscope
 * values (the gyroscope words are part of the burst but unused, like the
 * Python node).
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const REG_PWR_MGMT_1 = 0x6b;
const REG_ACCEL_XOUT_H = 0x3b;

const ADDRESS = 0x68;
const LSB_PER_G = 16384.0;

/** The big-endian two's-complement word at raw[i], raw[i + 1]. */
function s16be(raw, i) {
  const value = (raw[i] << 8) | raw[i + 1];
  return value > 32767 ? value - 65536 : value;
}

/*
 * The 14-byte ACCEL_XOUT_H burst -> { x, y, z, temperature }: axis g
 * values plus the die temperature in °C.
 */
function decodeAccelTemp(raw) {
  return {
    x: s16be(raw, 0) / LSB_PER_G,
    y: s16be(raw, 2) / LSB_PER_G,
    z: s16be(raw, 4) / LSB_PER_G,
    temperature: s16be(raw, 6) / 340.0 + 36.53,
  };
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

class MPU6050Driver {
  /** Open the sensor on /dev/i2c-<bus> at 0x68 and wake it from sleep. */
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
    // Wake the chip (it powers up in sleep mode).
    await bus.writeByte(ADDRESS, REG_PWR_MGMT_1, 0x00);
    return new MPU6050Driver(bus);
  }

  constructor(bus) {
    this._bus = bus;
  }

  /** Die temperature and roll/pitch/g-force from one burst read. */
  async read() {
    const buf = Buffer.alloc(14);
    await this._bus.readI2cBlock(ADDRESS, REG_ACCEL_XOUT_H, 14, buf);
    const { x, y, z, temperature } = decodeAccelTemp(buf);
    return { temperature, ...axesToOrientation(x, y, z) };
  }
}

module.exports = { MPU6050Driver, s16be, decodeAccelTemp, axesToOrientation };
