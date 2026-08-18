# Grove NFC Tag Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a Grove NFC Tag — a **passive
dual-interface EEPROM** (ST M24LR64E-R, 8 KB). The board is not a
reader: a phone or any ISO 15693 NFC writer stores an NDEF message on
it over RF, and this node reads the same memory over I2C. It is a
*push* node: it polls the EEPROM once a second and pushes one JSON
message over a **WebSocket** (`ws://`, port 9132, `X-Api-Key` checked
on the handshake) whenever the content changes. The app discovers it
via UDP broadcast on port 9133. Over BLE the node advertises the Sensor
Tester GATT service instead and sends each change as a notification.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_nfctag/` and supports the same two transports (Wi-Fi
and BLE).

## Protocol

One message per content change — and, because the tag holds state, the
current content is also sent to every client right after it connects
(over BLE it is additionally served on a read of the data
characteristic):

```json
{"kind": "text", "value": "Hello"}
{"kind": "uri",  "value": "https://seeed.cc"}
{"kind": "data", "value": "DEADBEEF"}
{"kind": "empty"}
```

The node parses the first record of the NDEF message (Type 5 tag
layout: capability container + `0x03` TLV): well-known Text and URI
records are decoded, anything else is reported as an uppercase hex dump
capped at 64 bytes. A blank or NDEF-less tag reports `empty`.

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket push server on port 9132 plus UDP
  discovery on port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery. Only
  one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
tag hardware — it then cycles through a text, a URI and an empty
content every 15 seconds (works with both transports). Useful for
testing the app against a node on any machine.

## Hardware

- [Grove NFC Tag](https://wiki.seeedstudio.com/Grove_NFC_Tag/)
  (ST M24LR64E-R), user memory on I2C address `0x53`
- Write the tag with any NFC-capable phone (e.g. Android with an
  NDEF-writer app, "Type 5 / ISO 15693" tag)

### Wiring (I2C)

| Pi Pin | Board Pin |
|--------|-----------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

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
| `i2c_bus` | `1` | I2C bus the tag is wired to |
| `i2c_address` | `"0x53"` | The M24LR64E user-memory address |
| `transport` | `wifi` | `wifi` (WebSocket + UDP discovery) or `ble` |
| `emulation` | `false` | Cycle generated contents without hardware |

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

The current content arrives immediately; write the tag with a phone and
a new `{"kind": ...}` message appears within a second.

## Running as a Service (optional)

```ini
# /etc/systemd/system/sensor-tester-nfctag.service
[Unit]
Description=Sensor Tester NFC tag node
After=network-online.target

[Service]
WorkingDirectory=/home/pi/nfctag
ExecStart=/home/pi/nfctag/venv/bin/python3 sensor_node.py
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

For the BLE transport add `After=bluetooth.target` and run as
`User=root`.
