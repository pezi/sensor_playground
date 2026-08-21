'use strict';
/*
 * AT24C128 driver — the text-region record format plus the I2C access to
 * a 128 Kbit (16 KB) serial EEPROM with 16-bit memory addressing, a
 * faithful port of the Python node's At24c128Eeprom and EmulatedEeprom.
 *
 * The record lives at offset 0: magic 'S' 'P', a u16 big-endian byte
 * length (max 512), then the UTF-8 text. A chip without the magic (e.g.
 * factory-fresh, all 0xFF) reads as an empty text rather than as garbage.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

// EEPROM text region: 2-byte magic + 2-byte big-endian length + up to
// 512 bytes of UTF-8 text.
const MAGIC = Buffer.from('SP', 'latin1');
const HEADER_SIZE = 4;
const TEXT_MAX_BYTES = 512;

// AT24C128 write page; a write transaction must not cross a page boundary
// or it wraps around inside the page.
const PAGE_SIZE = 64;

// Bytes per I2C transaction, safe on every adapter.
const IO_CHUNK = 32;

// Worst-case internal write cycle per the datasheet is 5 ms.
const WRITE_CYCLE_MS = 6;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// -- Record encoding ---------------------------------------------------------

/*
 * Returns the bytes stored at offset 0 for [textBytes]: the magic, the
 * big-endian byte length and the UTF-8 text itself.
 */
function encodeRecord(textBytes) {
  const header = Buffer.alloc(HEADER_SIZE);
  MAGIC.copy(header, 0);
  header.writeUInt16BE(textBytes.length, 2);
  return Buffer.concat([header, Buffer.from(textBytes)]);
}

/*
 * Returns the stored text length read from a header, or -1 when the chip
 * holds no usable record: a missing magic or an implausible length reads
 * as an empty text rather than as garbage.
 */
function recordLength(header) {
  if (header.length < HEADER_SIZE || header.compare(MAGIC, 0, 2, 0, 2) !== 0) return -1;
  const length = header.readUInt16BE(2);
  return length > TEXT_MAX_BYTES ? -1 : length;
}

/*
 * Parses a whole record (header + payload) into the stored text — the
 * pure counterpart of At24c128Eeprom.readText.
 */
function decodeRecord(record) {
  const length = recordLength(record);
  if (length <= 0) return '';
  // A truncated dump reports what is there; on the chip the read always
  // returns the full declared length.
  return record.subarray(HEADER_SIZE, Math.min(HEADER_SIZE + length, record.length)).toString('utf8');
}

// -- Hardware ----------------------------------------------------------------

/** Reads and writes the AT24C128 text region over I2C (16-bit addressing). */
class At24c128Eeprom {
  /*
   * Open the chip on /dev/i2c-<bus> and read it once, failing fast on a
   * wiring/address problem (like the Python node's constructor).
   */
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
    const eeprom = new At24c128Eeprom(bus, address);
    await eeprom.readText();
    return eeprom;
  }

  constructor(bus, address) {
    this._bus = bus;
    this._address = address;
  }

  /*
   * Random-reads [length] bytes starting at [addr]. Each chunk is a
   * two-byte memory-address write followed by a sequential read; the
   * Python node uses one combined i2c_rdwr transaction, but the
   * AT24C128's address counter survives the stop condition in between,
   * so the bytes read are the same.
   */
  async _read(addr, length) {
    const data = Buffer.alloc(length);
    for (let offset = 0; offset < length; offset += IO_CHUNK) {
      const at = addr + offset;
      const count = Math.min(IO_CHUNK, length - offset);
      await this._bus.i2cWrite(this._address, 2, Buffer.from([at >> 8, at & 0xff]));
      await this._bus.i2cRead(this._address, count, data.subarray(offset, offset + count));
    }
    return data;
  }

  /*
   * Writes [data] starting at [addr], splitting the transactions so none
   * crosses a 64-byte page boundary, and waiting out the chip's internal
   * write cycle after each one.
   */
  async _write(addr, data) {
    let offset = 0;
    while (offset < data.length) {
      const at = addr + offset;
      const count = Math.min(IO_CHUNK, PAGE_SIZE - (at % PAGE_SIZE), data.length - offset);
      const frame = Buffer.concat([
        Buffer.from([at >> 8, at & 0xff]),
        data.subarray(offset, offset + count),
      ]);
      await this._bus.i2cWrite(this._address, frame.length, frame);
      await sleep(WRITE_CYCLE_MS);
      offset += count;
    }
  }

  /*
   * Reads the stored text from the chip. A missing magic or an
   * implausible length reads as an empty text.
   */
  async readText() {
    const header = await this._read(0, HEADER_SIZE);
    const length = recordLength(header);
    if (length <= 0) return '';
    return (await this._read(HEADER_SIZE, length)).toString('utf8');
  }

  /** Stores the UTF-8 [textBytes] (header + payload) on the chip. */
  async writeText(textBytes) {
    await this._write(0, encodeRecord(textBytes));
  }

  async close() {
    await this._bus.close();
  }
}

// -- Emulation ---------------------------------------------------------------

/** Keeps the text in memory instead of driving hardware. */
class EmulatedEeprom {
  constructor() {
    this._text = 'Hello from the emulated EEPROM';
  }

  /** Returns the fake stored text. */
  async readText() {
    return this._text;
  }

  /** Stores the UTF-8 [textBytes] in memory. */
  async writeText(textBytes) {
    this._text = Buffer.from(textBytes).toString('utf8');
    console.log(`[emulation] stored ${textBytes.length} bytes`);
  }

  async close() {}
}

module.exports = {
  HEADER_SIZE,
  TEXT_MAX_BYTES,
  encodeRecord,
  recordLength,
  decodeRecord,
  At24c128Eeprom,
  EmulatedEeprom,
};
