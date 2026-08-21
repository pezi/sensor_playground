# AT24C128 EEPROM Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with an **AT24C128 serial
EEPROM** (128 Kbit / 16 KB, I2C address `0x50`) — the *actuator* variant
of the interface. The app stores a short text on the chip and reads it
back at any time; the text survives power cycles of both ends. It serves
a **WebSocket** (`ws://`, port 9132, `X-Api-Key` handshake header)
instead of HTTPS REST, plus UDP discovery on port 9133.

It is the Node.js counterpart of
[`../../python/at24c128/`](../../python/at24c128/) and speaks the
identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Protocol

The node is the single source of truth: after a write it reads the chip
back and reports the *stored* text — also on connect, so the app opens
with what the chip actually holds:

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"write": "Hello"}` | store the text on the chip |
| app → node | `{"read": true}` | re-read the chip and push |
| node → app | `{"text": "Hello"}` | stored text, read back from the chip |

## EEPROM layout

The text lives at the start of the chip: magic `'S' 'P'`, a u16
big-endian byte length (max 512), then the UTF-8 text. A chip without
the magic (e.g. factory-fresh, all `0xFF`) reads as an empty text. The
app limits the text to 128 characters; 512 bytes fit that even when
every character encodes to 4 UTF-8 bytes.

Writes are split so that no I2C transaction crosses the chip's 64-byte
page boundary, and each one waits out the 5 ms internal write cycle.

## Hardware

- An AT24C128 EEPROM breakout (or the bare chip) —
  [datasheet](https://ww1.microchip.com/downloads/en/DeviceDoc/doc0670.pdf)

### Wiring (I2C)

| Pi Pin | Board Pin |
|--------|-----------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND (and A0–A2, WP for address 0x50, writable) |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Most breakout boards carry pull-up resistors on SDA/SCL; add 4.7 kΩ
pull-ups to 3.3V when wiring a bare chip.

Enable I2C: `sudo raspi-config` → Interface Options → I2C.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, i2c_bus
```

`i2c-bus` is an *optional* dependency with a native build; a failed build
does not stop `npm install`, and emulation mode works without it.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
chip — the text then lives in memory only. Works on any machine
(macOS/Windows included).

## Usage

```bash
node sensor_node.js
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
# then type: {"write": "Hello EEPROM"}
```

The stored text arrives immediately. Type `{"write": "Hello EEPROM"}`
and press enter — the node stores it and echoes
`{"text": "Hello EEPROM"}` read back from the chip. Restart the node,
reconnect, and the same text prints again.

`npm test` (or `node test_driver.js`) checks the EEPROM record encoding
(magic, length header, UTF-8 payload, absent magic, over-long length)
against the Python node's behavior.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-at24c128-node.service`.

## Deviations from the Python node

- BLE is not implemented; `"transport": "ble"` warns and serves the
  WebSocket instead.
- The chip is read with separate address-write/read transactions instead
  of the Python node's combined `i2c_rdwr` transaction — the AT24C128's
  address counter survives the stop condition in between, so the bytes
  read are the same. Writes are single transactions, as in Python.
- I2C access is asynchronous here, so commands are serialized through one
  promise chain instead of running inline. A failed read or write is
  printed and the last known text is kept; the Python node lets the
  exception end the process.
- Text with invalid UTF-8 is decoded by `Buffer.toString('utf8')`, which
  may collapse a truncated multi-byte sequence into a single U+FFFD where
  Python's `errors="replace"` produces one per byte.
- Everything else — config schema, wire protocol, EEPROM layout, page
  and chunk sizes, write-cycle delay, the read-back after every command
  and the emulation text — matches the Python node.
