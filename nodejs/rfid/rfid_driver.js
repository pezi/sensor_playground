'use strict';
/*
 * RDM630 frame parsing for the Grove 125KHz RFID Reader — portable, so it
 * can be unit-tested on any machine.
 *
 * Frame format (reader TX, jumper on UART mode — not Wiegand):
 *     STX 0x02 | 10 ASCII-hex data chars | 2 ASCII-hex checksum chars | ETX 0x03
 * The checksum byte is the XOR of the five data bytes.
 */

const STX = 0x02;
const ETX = 0x03;
const FRAME_HEX_CHARS = 12; // 10 data chars + 2 checksum chars

/** XOR of the five data bytes must equal the checksum byte. */
function checksumOk(frame) {
  const values = [];
  for (let i = 0; i < FRAME_HEX_CHARS; i += 2) {
    values.push(parseInt(frame.slice(i, i + 2), 16));
  }
  let checksum = 0;
  for (const value of values.slice(0, 5)) checksum ^= value;
  return checksum === values[5];
}

/** Byte-wise state machine for RDM630-style frames. */
class RdmFrameParser {
  constructor() {
    this._frame = null; // null = waiting for STX, else collected hex chars
  }

  /**
   * Advance the state machine by one byte; returns a validated 10-char
   * hex tag whenever a byte completes a frame, else null.
   */
  feed(byte) {
    if (byte === STX) {
      this._frame = ''; // resync, also on a second STX
      return null;
    }
    if (this._frame === null) return null; // noise outside a frame
    if (byte === ETX) {
      const frame = this._frame;
      this._frame = null;
      if (frame.length === FRAME_HEX_CHARS && checksumOk(frame)) {
        return frame.slice(0, 10).toUpperCase();
      }
      return null;
    }
    const char = String.fromCharCode(byte);
    if (/[0-9a-fA-F]/.test(char)) {
      this._frame += char;
      if (this._frame.length > FRAME_HEX_CHARS) {
        this._frame = null; // overflow: wait for the next STX
      }
    } else {
      this._frame = null; // non-hex noise mid-frame
    }
    return null;
  }
}

module.exports = { RdmFrameParser, checksumOk, STX, ETX, FRAME_HEX_CHARS };
