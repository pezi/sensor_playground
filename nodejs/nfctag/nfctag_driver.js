'use strict';
/*
 * Grove NFC Tag driver — NDEF parsing plus M24LR64E-R user-memory access,
 * a faithful port of the Python node's parse_ndef_area/_parse_ndef_record
 * and M24lr64Tag: the tag is a passive dual-interface EEPROM whose NDEF
 * area (Type 5 capability container + TLV stream) is scanned over I2C
 * with 16-bit memory addressing. Anything that fails to parse is reported
 * as a hex dump rather than dropped, so the app always sees that
 * *something* was written.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

// Bytes of the EEPROM scanned for the NDEF message (CC + TLV area).
const SCAN_LENGTH = 256;

// Cap for hex dumps of unparseable payloads (bytes before hex encoding).
const DATA_HEX_CAP = 64;

// Bytes per I2C transaction, safe on every adapter.
const CHUNK = 32;

// NFC Forum URI record prefix codes (the common subset; the same table
// lives in the ESP32 sketch — keep them identical).
const URI_PREFIXES = {
  0x00: '',
  0x01: 'http://www.',
  0x02: 'https://www.',
  0x03: 'http://',
  0x04: 'https://',
  0x05: 'tel:',
  0x06: 'mailto:',
};

// -- NDEF parsing ------------------------------------------------------------

/*
 * Parses the scanned EEPROM area into a {kind, value} content object
 * ({kind: 'empty'} carries no value). The area starts with the Type 5
 * capability container (magic 0xE1/0xE2), followed by a TLV stream in
 * which 0x03 marks the NDEF message.
 */
function parseNdefArea(data) {
  if (data.length < 4 || (data[0] !== 0xe1 && data[0] !== 0xe2)) {
    if (data.every((byte) => byte === 0x00 || byte === 0xff)) {
      return { kind: 'empty' };
    }
    return dataContent(data);
  }

  let offset = 4; // first byte after the 4-byte capability container
  while (offset < data.length) {
    const tlv = data[offset];
    if (tlv === 0x00) { // padding
      offset += 1;
      continue;
    }
    if (tlv === 0xfe) { // terminator: no NDEF TLV found
      return { kind: 'empty' };
    }
    if (tlv !== 0x03) { // unknown TLV: skip it (1-byte length format)
      if (offset + 1 >= data.length) return { kind: 'empty' };
      offset += 2 + data[offset + 1];
      continue;
    }
    // NDEF message TLV: 1-byte length, or 0xFF + 2-byte big-endian.
    if (offset + 1 >= data.length) return { kind: 'empty' };
    let length = data[offset + 1];
    offset += 2;
    if (length === 0xff) {
      if (offset + 2 > data.length) return { kind: 'empty' };
      length = (data[offset] << 8) | data[offset + 1];
      offset += 2;
    }
    if (length === 0) return { kind: 'empty' };
    const message = data.subarray(offset, offset + length);
    if (message.length < length) {
      return dataContent(message); // truncated by the scan window
    }
    return parseNdefRecord(message);
  }
  return { kind: 'empty' };
}

/*
 * Parses the first record of an NDEF message. An index the record's
 * declared lengths put outside the message is the Python node's
 * IndexError: the whole message is reported as a hex dump.
 */
function parseNdefRecord(message) {
  if (message.length < 2) return dataContent(message);
  const flags = message[0];
  const tnf = flags & 0x07;
  const shortRecord = (flags & 0x10) !== 0;
  const hasId = (flags & 0x08) !== 0;
  const typeLength = message[1];
  let offset = 2;
  let payloadLength;
  if (shortRecord) {
    if (offset >= message.length) return dataContent(message);
    payloadLength = message[offset];
    offset += 1;
  } else {
    payloadLength = beUint(message.subarray(offset, offset + 4));
    offset += 4;
  }
  let idLength = 0;
  if (hasId) {
    if (offset >= message.length) return dataContent(message);
    idLength = message[offset];
    offset += 1;
  }
  const recordType = message.subarray(offset, offset + typeLength).toString('latin1');
  offset += typeLength + idLength;
  const payload = message.subarray(offset, offset + payloadLength);
  if (payload.length < payloadLength) return dataContent(payload);

  if (tnf === 0x01 && recordType === 'T') {
    if (payload.length === 0) return dataContent(message);
    const status = payload[0];
    const langLength = status & 0x3f;
    const text = payload.subarray(1 + langLength);
    const value = (status & 0x80) !== 0 ? decodeUtf16(text) : text.toString('utf8');
    return { kind: 'text', value };
  }
  if (tnf === 0x01 && recordType === 'U') {
    if (payload.length === 0) return dataContent(message);
    const prefix = URI_PREFIXES[payload[0]] ?? '';
    return { kind: 'uri', value: prefix + payload.subarray(1).toString('utf8') };
  }
  return dataContent(payload);
}

/** Hex-dump fallback for content that is not a text or URI record. */
function dataContent(data) {
  return { kind: 'data', value: data.subarray(0, DATA_HEX_CAP).toString('hex').toUpperCase() };
}

/** Python's int.from_bytes(b, 'big') — tolerates short slices. */
function beUint(bytes) {
  let value = 0;
  for (const byte of bytes) value = value * 256 + byte;
  return value;
}

/*
 * Decodes UTF-16 like Python's "utf-16" codec: a BOM selects the byte
 * order, without one little-endian is assumed; a trailing odd byte
 * becomes U+FFFD.
 */
function decodeUtf16(buf) {
  let littleEndian = true;
  if (buf.length >= 2) {
    if (buf[0] === 0xff && buf[1] === 0xfe) {
      buf = buf.subarray(2);
    } else if (buf[0] === 0xfe && buf[1] === 0xff) {
      littleEndian = false;
      buf = buf.subarray(2);
    }
  }
  const odd = buf.length % 2 === 1;
  let body = Buffer.from(buf.subarray(0, buf.length - (odd ? 1 : 0)));
  if (!littleEndian) body.swap16();
  return body.toString('utf16le') + (odd ? '�' : '');
}

// -- Tag hardware ------------------------------------------------------------

/** Reads the M24LR64E-R user memory over I2C (16-bit addressing). */
class M24lr64Tag {
  /*
   * Open the tag on /dev/i2c-<bus> and read it once, failing fast on a
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
    const tag = new M24lr64Tag(bus, address);
    await tag.readContent();
    return tag;
  }

  constructor(bus, address) {
    this._bus = bus;
    this._address = address;
  }

  /*
   * Scans the NDEF area and returns the parsed content. Each chunk is a
   * two-byte memory-address write followed by a sequential read; the
   * Python node uses one combined i2c_rdwr transaction, but the M24LR64's
   * address pointer survives the stop condition in between, so the bytes
   * read are the same.
   */
  async readContent() {
    const data = Buffer.alloc(SCAN_LENGTH);
    for (let offset = 0; offset < SCAN_LENGTH; offset += CHUNK) {
      await this._bus.i2cWrite(this._address, 2, Buffer.from([offset >> 8, offset & 0xff]));
      await this._bus.i2cRead(this._address, CHUNK, data.subarray(offset, offset + CHUNK));
    }
    return parseNdefArea(data);
  }

  async close() {
    await this._bus.close();
  }
}

/** Cycles through generated tag contents without hardware. */
class EmulatedNfcTag {
  static CONTENTS = [
    { kind: 'text', value: 'Hello from Sensor Playground' },
    { kind: 'uri', value: 'https://wiki.seeedstudio.com/Grove_NFC_Tag/' },
    { kind: 'empty' },
  ];

  constructor() {
    this._index = 0;
    this._nextAt = Date.now() + 15000;
  }

  /** Returns the current fake content, advancing every 15 seconds. */
  async readContent() {
    if (Date.now() >= this._nextAt) {
      this._index = (this._index + 1) % EmulatedNfcTag.CONTENTS.length;
      this._nextAt = Date.now() + 15000;
    }
    return EmulatedNfcTag.CONTENTS[this._index];
  }

  async close() {}
}

module.exports = { SCAN_LENGTH, DATA_HEX_CAP, parseNdefArea, M24lr64Tag, EmulatedNfcTag };
