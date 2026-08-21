# AT24C128 EEPROM Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with an **AT24C128 serial
EEPROM** (128 Kbit / 16 KB, I2C address `0x50`) — the *actuator* variant
of the interface. The app stores a short text on the chip and reads it
back at any time; the text survives power cycles of both ends. It serves
a **WebSocket** (`ws://`, port 9132, `X-Api-Key` handshake header)
instead of HTTPS REST, plus UDP discovery on port 9133.

It is the Rust counterpart of
[`../../python/at24c128/`](../../python/at24c128/) and speaks the
identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).
  Over BLE the stored text arrives as a notify on the data
  characteristic carrying the same JSON, and a write is staged in
  offset-addressed binary chunks on the command characteristic.

## Protocol

The node is the single source of truth: after a write it reads the chip
back and reports the *stored* text — also on connect, so the app opens
with what the chip actually holds:

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"write": "Hello"}` | store the text on the chip |
| app → node | `{"read": true}` | re-read the chip and push |
| node → app | `{"text": "Hello"}` | stored text, read back from the chip |

Over BLE the text notify carries the same `{"text": …}` JSON, but a
write is staged in offset-addressed binary chunks on the command
characteristic (like the SSD1306 bitmap), because a text may not fit in
a single ATT write:

| Packet | Meaning |
|--------|---------|
| `0x01 <offset:u16 BE> <bytes…>` | stage a chunk of UTF-8 text |
| `0x02 <length:u16 BE>` | store the staged text (refused unless exactly `length` bytes are staged) |
| `0x03` | re-read the chip and push |

Chunks must arrive contiguously; staging offset 0 starts a new transfer,
so a dropped packet refuses the write instead of storing a torn text.

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

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace. On a non-Pi development host, build from this folder
normally:

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key, i2c_bus
```

For a native Raspberry Pi build, `rustc 1.97.1` can crash with
`SIGSEGV`. Use the verified Rust 1.96.0 workaround and compile one job
at a time (see the [diagnosis and power checks](../bme680/README.md#native-raspberry-pi-builds)):

```bash
rustup toolchain install 1.96.0 --profile minimal
rustup override set 1.96.0
cargo build --release -j 1
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
chip — the text then lives in memory only (works with both transports).
Works on any machine (macOS/Windows included).

## Usage

```bash
../target/release/sensor_node_at24c128
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

`cargo test` checks the EEPROM record encoding (magic, length header,
UTF-8 payload, absent magic, over-long length) and the BLE chunk
assembler (contiguity, restart at offset 0, length mismatch, region
overrun) against the Python node's behavior.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-at24c128-rust.service`.
For the BLE transport add `After=bluetooth.target` and run as
`User=root`.

## Deviations from the Python node

- The chip is read with separate address-write/read transactions instead
  of the Python node's combined `i2c_rdwr` transaction — the AT24C128's
  address counter survives the stop condition in between, so the bytes
  read are the same. Writes are single transactions, as in Python.
- An I2C error during a read or write is printed and the last known text
  is kept; the Python node lets the exception end the process.
- Text with invalid UTF-8 gets one U+FFFD per invalid byte, matching
  Python's `errors="replace"`.
- Everything else — config schema, both wire protocols, the BLE chunk
  staging rules, EEPROM layout, page and chunk sizes, write-cycle delay,
  the read-back after every command and the emulation text — matches the
  Python node.
