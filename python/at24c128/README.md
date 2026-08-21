# AT24C128 EEPROM Node for Sensor Playground (Python)

This Python script implements the Sensor Playground sensor interface on
single-board computers with an **AT24C128 serial EEPROM** (128 Kbit /
16 KB, I2C address `0x50`). It is an *actuator* node: the app stores a
short text on the chip and reads it back at any time; the text survives
power cycles of both ends. Over Wi-Fi the node serves a **WebSocket**
(`ws://`, port 9132, `X-Api-Key` checked on the handshake) and is
discovered via UDP broadcast on port 9133. Over BLE the node advertises
the Sensor Playground GATT service instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_at24c128/` and supports the same two transports
(Wi-Fi and BLE).

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

## EEPROM layout

The text lives at the start of the chip: magic `'S' 'P'`, a u16
big-endian byte length (max 512), then the UTF-8 text. A chip without
the magic (e.g. factory-fresh, all `0xFF`) reads as an empty text. The
app limits the text to 128 characters; 512 bytes fit that even when
every character encodes to 4 UTF-8 bytes.

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket server on port 9132 plus UDP discovery
  on port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery. Only
  one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
chip — the text then lives in memory only (works with both transports).
Useful for testing the app against a node on any machine.

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

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `api_key` | — | Shared key the app must present (min. 8 characters) |
| `hostname` | `""` | Name shown in the app; empty = system hostname |
| `i2c_bus` | `1` | I2C bus the chip is wired to |
| `i2c_address` | `"0x50"` | I2C address (0x50–0x57, set by the A0–A2 pins) |
| `transport` | `wifi` | `wifi` (WebSocket + UDP discovery) or `ble` |
| `emulation` | `false` | Keep the text in memory without hardware |

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
cp config.example.json config.json   # then edit it
```

## Usage

```bash
python3 sensor_node.py
```

## Testing

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

The stored text arrives immediately. Type `{"write": "Hello EEPROM"}`
and press enter — the node stores it and echoes
`{"text": "Hello EEPROM"}` read back from the chip. Restart the node,
reconnect, and the same text prints again.

## Running as a Service (optional)

```ini
# /etc/systemd/system/sensor-playground-at24c128.service
[Unit]
Description=Sensor Playground AT24C128 EEPROM node
After=network-online.target

[Service]
WorkingDirectory=/home/pi/at24c128
ExecStart=/home/pi/at24c128/venv/bin/python3 sensor_node.py
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

For the BLE transport add `After=bluetooth.target` and run as
`User=root`.
