'use strict';
/*
 * PAJ7620U2 gesture driver, ported from Seeed's grove.py
 * grove_gesture_sensor.py (MIT) — the library the Python node uses — so
 * the detected gestures match it: the same wake-up retry, the same init
 * register table (both banks) and the same two-phase flag decoding with
 * the 0.8 s entry / 1.0 s quit waits around the combined forward/backward
 * gestures.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const ADDRESS = 0x73; // I2C address (equals the device ID, as in grove.py)
const REG_BANK_SEL = 0xef; // register bank select
const REG_FLAG_0 = 0x43; // gesture detection flags, bank 0
const REG_FLAG_1 = 0x44; // wave flag, bank 0

/*
 * grove.py's GES_ENTRY_TIME / GES_QUIT_TIME: after a directional flag the
 * driver waits and re-reads to catch a combined forward/backward, and
 * backs off after a forward/backward so the hand can leave.
 */
const GES_ENTRY_TIME_MS = 800;
const GES_QUIT_TIME_MS = 1000;

const WAKE_ATTEMPTS = 3;

// Gesture detection flags (register 0x43; the wave flag lives in 0x44).
const GES_RIGHT_FLAG = 1 << 0;
const GES_LEFT_FLAG = 1 << 1;
const GES_UP_FLAG = 1 << 2;
const GES_DOWN_FLAG = 1 << 3;
const GES_FORWARD_FLAG = 1 << 4;
const GES_BACKWARD_FLAG = 1 << 5;
const GES_CLOCKWISE_FLAG = 1 << 6;
const GES_ANTI_CLOCKWISE_FLAG = 1 << 7;
const GES_WAVE_FLAG = 1 << 0;

/*
 * grove.py's return_gesture() codes mapped to the app's Gesture enum
 * names (0 = no gesture).
 */
const GESTURES = {
  1: 'forward',
  2: 'backward',
  3: 'right',
  4: 'left',
  5: 'up',
  6: 'down',
  7: 'clockwise',
  8: 'antiClockwise',
  9: 'wave',
};

/*
 * grove.py's initRegisterArray verbatim: the undocumented power-on
 * configuration from Seeed's reference driver, including the bank switch
 * ([0xEF, 0x01]) in the middle.
 */
const INIT_REGISTERS = [
  [0xef, 0x00], [0x32, 0x29], [0x33, 0x01], [0x34, 0x00], [0x35, 0x01],
  [0x36, 0x00], [0x37, 0x07], [0x38, 0x17], [0x39, 0x06], [0x3a, 0x12],
  [0x3f, 0x00], [0x40, 0x02], [0x41, 0xff], [0x42, 0x01], [0x46, 0x2d],
  [0x47, 0x0f], [0x48, 0x3c], [0x49, 0x00], [0x4a, 0x1e], [0x4b, 0x00],
  [0x4c, 0x20], [0x4d, 0x00], [0x4e, 0x1a], [0x4f, 0x14], [0x50, 0x00],
  [0x51, 0x10], [0x52, 0x00], [0x5c, 0x02], [0x5d, 0x00], [0x5e, 0x10],
  [0x5f, 0x3f], [0x60, 0x27], [0x61, 0x28], [0x62, 0x00], [0x63, 0x03],
  [0x64, 0xf7], [0x65, 0x03], [0x66, 0xd9], [0x67, 0x03], [0x68, 0x01],
  [0x69, 0xc8], [0x6a, 0x40], [0x6d, 0x04], [0x6e, 0x00], [0x6f, 0x00],
  [0x70, 0x80], [0x71, 0x00], [0x72, 0x00], [0x73, 0x00], [0x74, 0xf0],
  [0x75, 0x00], [0x80, 0x42], [0x81, 0x44], [0x82, 0x04], [0x83, 0x20],
  [0x84, 0x20], [0x85, 0x00], [0x86, 0x10], [0x87, 0x00], [0x88, 0x05],
  [0x89, 0x18], [0x8a, 0x10], [0x8b, 0x01], [0x8c, 0x37], [0x8d, 0x00],
  [0x8e, 0xf0], [0x8f, 0x81], [0x90, 0x06], [0x91, 0x06], [0x92, 0x1e],
  [0x93, 0x0d], [0x94, 0x0a], [0x95, 0x0a], [0x96, 0x0c], [0x97, 0x05],
  [0x98, 0x0a], [0x99, 0x41], [0x9a, 0x14], [0x9b, 0x0a], [0x9c, 0x3f],
  [0x9d, 0x33], [0x9e, 0xae], [0x9f, 0xf9], [0xa0, 0x48], [0xa1, 0x13],
  [0xa2, 0x10], [0xa3, 0x08], [0xa4, 0x30], [0xa5, 0x19], [0xa6, 0x10],
  [0xa7, 0x08], [0xa8, 0x24], [0xa9, 0x04], [0xaa, 0x1e], [0xab, 0x1e],
  [0xcc, 0x19], [0xcd, 0x0b], [0xce, 0x13], [0xcf, 0x64], [0xd0, 0x21],
  [0xd1, 0x0f], [0xd2, 0x88], [0xe0, 0x01], [0xe1, 0x04], [0xe2, 0x41],
  [0xe3, 0xd6], [0xe4, 0x00], [0xe5, 0x0c], [0xe6, 0x0a], [0xe7, 0x00],
  [0xe8, 0x00], [0xe9, 0x00], [0xee, 0x07], [0xef, 0x01], [0x00, 0x1e],
  [0x01, 0x1e], [0x02, 0x0f], [0x03, 0x10], [0x04, 0x02], [0x05, 0x00],
  [0x06, 0xb0], [0x07, 0x04], [0x08, 0x0d], [0x09, 0x0e], [0x0a, 0x9c],
  [0x0b, 0x04], [0x0c, 0x05], [0x0d, 0x0f], [0x0e, 0x02], [0x0f, 0x12],
  [0x10, 0x02], [0x11, 0x02], [0x12, 0x00], [0x13, 0x01], [0x14, 0x05],
  [0x15, 0x07], [0x16, 0x05], [0x17, 0x07], [0x18, 0x01], [0x19, 0x04],
  [0x1a, 0x05], [0x1b, 0x0c], [0x1c, 0x2a], [0x1d, 0x01], [0x1e, 0x00],
  [0x21, 0x00], [0x22, 0x00], [0x23, 0x00], [0x25, 0x01], [0x26, 0x00],
  [0x27, 0x39], [0x28, 0x7f], [0x29, 0x08], [0x30, 0x03], [0x31, 0x00],
  [0x32, 0x1a], [0x33, 0x1a], [0x34, 0x07], [0x35, 0x07], [0x36, 0x01],
  [0x37, 0xff], [0x38, 0x36], [0x39, 0x07], [0x3a, 0x00], [0x3e, 0xff],
  [0x3f, 0x00], [0x40, 0x77], [0x41, 0x40], [0x42, 0x00], [0x43, 0x30],
  [0x44, 0xa0], [0x45, 0x5c], [0x46, 0x00], [0x47, 0x00], [0x48, 0x58],
  [0x4a, 0x1e], [0x4b, 0x1e], [0x4c, 0x00], [0x4d, 0x00], [0x4e, 0xa0],
  [0x4f, 0x80], [0x50, 0x00], [0x51, 0x00], [0x52, 0x00], [0x53, 0x00],
  [0x54, 0x00], [0x57, 0x80], [0x59, 0x10], [0x5a, 0x08], [0x5b, 0x94],
  [0x5c, 0xe8], [0x5d, 0x08], [0x5e, 0x3d], [0x5f, 0x99], [0x60, 0x45],
  [0x61, 0x40], [0x63, 0x2d], [0x64, 0x02], [0x65, 0x96], [0x66, 0x00],
  [0x67, 0x97], [0x68, 0x01], [0x69, 0xcd], [0x6a, 0x01], [0x6b, 0xb0],
  [0x6c, 0x04], [0x6d, 0x2c], [0x6e, 0x01], [0x6f, 0x32], [0x71, 0x00],
  [0x72, 0x01], [0x73, 0x35], [0x74, 0x00], [0x75, 0x33], [0x76, 0x31],
  [0x77, 0x01], [0x7c, 0x84], [0x7d, 0x03], [0x7e, 0x01],
];

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * grove.py's return_gesture(): read the flag register; a directional flag
 * is held for GES_ENTRY_TIME and re-read, because a hand entering the
 * field fires right/left/up/down before the driver can tell a
 * forward/backward push. Returns the gesture code (0 = none). Reading
 * (async readReg(reg)) and sleeping (async sleepMs(ms)) are injected so
 * the decoding is unit-testable.
 */
async function decodeGesture(readReg, sleepMs) {
  let data = await readReg(REG_FLAG_0);
  const directional = {
    [GES_RIGHT_FLAG]: 3,
    [GES_LEFT_FLAG]: 4,
    [GES_UP_FLAG]: 5,
    [GES_DOWN_FLAG]: 6,
  };
  if (data in directional) {
    const code = directional[data];
    await sleepMs(GES_ENTRY_TIME_MS);
    data = await readReg(REG_FLAG_0);
    if (data === GES_FORWARD_FLAG) {
      await sleepMs(GES_QUIT_TIME_MS);
      return 1;
    }
    if (data === GES_BACKWARD_FLAG) {
      await sleepMs(GES_QUIT_TIME_MS);
      return 2;
    }
    return code;
  }
  if (data === GES_FORWARD_FLAG) {
    await sleepMs(GES_QUIT_TIME_MS);
    return 1;
  }
  if (data === GES_BACKWARD_FLAG) {
    await sleepMs(GES_QUIT_TIME_MS);
    return 2;
  }
  if (data === GES_CLOCKWISE_FLAG) return 7;
  if (data === GES_ANTI_CLOCKWISE_FLAG) return 8;
  const data1 = await readReg(REG_FLAG_1);
  if (data1 === GES_WAVE_FLAG) return 9;
  return 0;
}

class PAJ7620Driver {
  /*
   * Open the sensor on /dev/i2c-<bus> at 0x73 and run the init sequence,
   * waking the sensor if it does not answer: the PAJ7620 keeps its I2C
   * interface powered down until bus activity wakes it, and does not
   * acknowledge the transaction that does the waking — so the first
   * register write of an unpoked sensor fails with EIO. Poke the bus,
   * give the sensor a moment, and try again (like the Python node).
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
    const driver = new PAJ7620Driver(bus);
    let lastError;
    for (let attempt = 0; attempt < WAKE_ATTEMPTS; attempt++) {
      try {
        await driver._init();
        return driver;
      } catch (err) {
        lastError = err;
      }
      if (attempt < WAKE_ATTEMPTS - 1) {
        // One throwaway transaction wakes the sensor; it is itself not
        // acknowledged, so its error is expected and ignored.
        await bus.sendByte(ADDRESS, 0x00).catch(() => {});
        await sleep(50);
      }
    }
    throw new Error(
      `PAJ7620 did not respond on i2c bus ${busNumber} (${lastError}). ` +
      'Check the wiring and that the sensor appears at 0x73.'
    );
  }

  constructor(bus) {
    this._bus = bus;
  }

  _writeReg(reg, value) {
    return this._bus.writeByte(ADDRESS, reg, value);
  }

  _readReg(reg) {
    return this._bus.readByte(ADDRESS, reg);
  }

  /*
   * grove.py's init(): select bank 0 (twice, the second select covers a
   * sensor that ignored the waking first one), verify the part id and
   * write the whole init register table.
   */
  async _init() {
    await sleep(1);
    await this._writeReg(REG_BANK_SEL, 0);
    await this._writeReg(REG_BANK_SEL, 0);
    const data0 = await this._readReg(0);
    await this._readReg(1);
    // grove.py only warns on an unexpected part id and carries on.
    if (data0 !== 0x20) {
      console.log('Error with sensor');
    } else {
      console.log('wake-up finish.');
    }
    for (const [reg, value] of INIT_REGISTERS) {
      await this._writeReg(reg, value);
    }
    await this._writeReg(REG_BANK_SEL, 0);
    console.log('Paj7620 initialize register finished.');
  }

  /** The detected gesture name, or null if nothing happened. */
  async readGesture() {
    const code = await decodeGesture((reg) => this._readReg(reg), sleep);
    return GESTURES[code] || null;
  }
}

module.exports = {
  PAJ7620Driver,
  GESTURES,
  decodeGesture,
  GES_ENTRY_TIME_MS,
  GES_QUIT_TIME_MS,
};
