# AT24C128 EEPROM Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with an **AT24C128 serial
EEPROM** (128 Kbit / 16 KB, I2C address `0x50`) — the *actuator* variant
of the interface. The app stores a short text on the chip and reads it
back at any time; the text survives power cycles of both ends. It serves
a **WebSocket** (`ws://`, port 9132, `X-Api-Key` handshake header)
instead of HTTPS REST, plus UDP discovery on port 9133.

It is the Go counterpart of
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

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_at24c128 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_at24c128 .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
chip — the text then lives in memory only. Works on any machine
(macOS/Windows included).

## Usage

```bash
./sensor_node_at24c128
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

`go test ./at24c128/` checks the EEPROM record encoding (magic, length
header, UTF-8 payload, absent magic, over-long length) against the
Python node's behavior.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-at24c128-go.service`.

## Deviations from the Python node

- BLE is not implemented; `"transport": "ble"` warns and serves the
  WebSocket instead.
- The chip is read with separate address-write/read transactions instead
  of the Python node's combined `i2c_rdwr` transaction — the AT24C128's
  address counter survives the stop condition in between, so the bytes
  read are the same. Writes are single transactions, as in Python.
- An I2C error during a read or write is printed and the last known text
  is kept; the Python node lets the exception end the process.
- Everything else — config schema, wire protocol, EEPROM layout, page
  and chunk sizes, write-cycle delay, the read-back after every command
  and the emulation text — matches the Python node.
