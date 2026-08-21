'use strict';
/*
 * SSD1306 128x64 OLED driver — a port of the Python node's, which is itself
 * a port of dart_periphery's SSD1306: the same init sequence, the same
 * horizontal-addressing window, and the same bitmap transposition.
 *
 * The bitmap transposition and the JSON command handling are
 * platform-neutral (and unit tested); only the I2C access requires the
 * `i2c-bus` package (Linux only), which is loaded lazily so that emulation
 * mode works without it.
 */

const WIDTH = 128;
const HEIGHT = 64;
// One bit per pixel: 1024 bytes for the whole panel.
const FRAME_SIZE = (WIDTH * HEIGHT) / 8;
// 16 bytes per row in the app's (horizontal) bitmap format.
const ROW_BYTES = WIDTH / 8;

const CONTROL_COMMAND = 0x00; // the byte that follows is a command
const CONTROL_DATA = 0x40; // the bytes that follow are display RAM

// The same init sequence the dart_periphery SSD1306 driver sends.
const INIT_SEQUENCE = [
  0xae, 0xd5, 0x80, 0xa8, 0x3f, 0xd3, 0x00, 0x40, 0x8d, 0x14, 0x20, 0x00,
  0xa1, 0xc8, 0xda, 0x12, 0x81, 0xcf, 0xd9, 0xf1, 0xdb, 0x40, 0xa4, 0xa6,
  0xaf,
];

/*
 * Transpose a horizontal MSB-first bitmap into SSD1306 page format.
 *
 * The app sends what https://javl.github.io/image2cpp/ calls "horizontal"
 * byte orientation: 16 bytes per row, the MSB of each byte is the leftmost
 * pixel. The panel wants 8 pages of 128 column bytes, each byte holding 8
 * *vertical* pixels with the topmost in the LSB — so every output byte is
 * assembled from one bit of eight different input rows.
 */
function toNativeFormat(data) {
  const buffer = Buffer.alloc(FRAME_SIZE);
  let count = 0;
  let index = 0;
  for (let page = 0; page < HEIGHT / 8; page++) {
    for (let columnByte = 0; columnByte < ROW_BYTES; columnByte++) {
      const pos = index + columnByte;
      for (let bit = 7; bit >= 0; bit--) {
        const mask = 1 << bit;
        let value = 0;
        for (let row = 0; row < 8; row++) {
          if (data[pos + ROW_BYTES * row] & mask) value |= 1 << row;
        }
        buffer[count] = value;
        count++;
      }
    }
    index += WIDTH;
  }
  return buffer;
}

/*
 * Reports what would be drawn instead of driving hardware. A 1024-byte
 * frame is meaningless on a console, so it reports the share of lit pixels
 * — enough to tell a blank frame from a drawn one.
 */
class EmulatedDisplay {
  async clear() {
    console.log('[emulation] display cleared');
  }

  async showBitmap(data) {
    let lit = 0;
    for (const value of toNativeFormat(data)) {
      for (let bit = 0; bit < 8; bit++) if (value & (1 << bit)) lit++;
    }
    console.log(`[emulation] display bitmap: ${lit} of ${WIDTH * HEIGHT} pixels lit`);
  }
}

class SSD1306Driver {
  /* Open the panel on /dev/i2c-<bus>, initialize it and blank it. */
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
    const display = new SSD1306Driver(bus, address);
    await display._write(CONTROL_COMMAND, Buffer.from(INIT_SEQUENCE));
    await display.clear();
    return display;
  }

  constructor(bus, address) {
    this._bus = bus;
    this._addr = address;
  }

  /* Send a control byte followed by its payload in one transfer. */
  async _write(control, data) {
    const message = Buffer.concat([Buffer.from([control]), data]);
    await this._bus.i2cWrite(this._addr, message.length, message);
  }

  /*
   * Point the panel's write cursor back at the top left by setting the full
   * column (0-127) and page (0-7) range for horizontal addressing mode.
   */
  _resetPosition() {
    return this._write(CONTROL_COMMAND, Buffer.from([0x21, 0x00, 0x7f, 0x22, 0x00, 0x07]));
  }

  /* Blank the display. */
  async clear() {
    await this._resetPosition();
    await this._write(CONTROL_DATA, Buffer.alloc(FRAME_SIZE));
  }

  /* Display a horizontal MSB-first bitmap (1024 bytes). */
  async showBitmap(data) {
    await this._resetPosition();
    await this._write(CONTROL_DATA, toNativeFormat(data));
  }
}

module.exports = {
  SSD1306Driver,
  EmulatedDisplay,
  toNativeFormat,
  FRAME_SIZE,
  WIDTH,
  HEIGHT,
  ROW_BYTES,
};
