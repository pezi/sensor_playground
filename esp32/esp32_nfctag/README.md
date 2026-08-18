# ESP32 Grove NFC Tag Node — Sensor Tester

ESP32 sketch for the [Grove NFC
Tag](https://wiki.seeedstudio.com/Grove_NFC_Tag/) — a **passive
dual-interface EEPROM** (ST M24LR64E-R, 8 KB). The board is not a
reader: a phone or any ISO 15693 NFC writer stores an NDEF message on
it over RF, and this node reads the same memory over I2C (user memory
at address `0x53`). It is a *push* node: it polls the NDEF area once a
second and pushes one JSON message whenever the content changes.
Because the tag holds state, the current content is also sent to every
client right after it connects.

## Transport

Selected at compile time via `ACTIVE_TRANSPORT` in the sketch (or the
`install.sh` argument):

| Transport | Wire protocol |
|-----------|---------------|
| `TRANSPORT_WIFI` | WebSocket server on port 9132 (`ws://`, `X-Api-Key` handshake header) + UDP discovery on port 9133 |
| `TRANSPORT_BLE` (default) | Sensor Tester GATT service; each change is one notify on the data characteristic, and a read serves the current content |

## Protocol (Wi-Fi)

- UDP discovery: the node answers a `SENSOR_TESTER` broadcast on port
  9133 with `{"type":"NFCTAG","host":"...","ip":"...","port":9132}`.
- WebSocket push — the current content on connect, then one message per
  change:

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

## Protocol (BLE)

| Characteristic | UUID | Purpose |
|----------------|------|---------|
| Service | `d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f` | advertised as `NFCTAG` |
| Data | `d1a51b00-0002-...` | READ + NOTIFY — the content JSON (chunked 0x1E framing, see `../common/sensor_ble_framing.h`); a read serves the current content |
| Auth | `d1a51b00-0003-...` | WRITE — the shared API key; a successful auth pushes the current content |

## Hardware Requirements

- ESP32 dev board
- Grove NFC Tag (M24LR64E-R)
- Any NFC-capable phone to write the tag (Type 5 / ISO 15693 NDEF)

### Wiring (I2C)

| ESP32 Pin | Board Pin |
|-----------|-----------|
| 3.3V | VCC |
| GND | GND |
| GPIO 21 (SDA) | SDA |
| GPIO 22 (SCL) | SCL |

## Software Requirements

1. Arduino IDE or `arduino-cli`
2. ESP32 board support (`esp32:esp32`)
3. Libraries: `ArduinoJson`, and for Wi-Fi `WebSockets`
   (Markus Sattler / Links2004)

### Quick install (arduino-cli)

```bash
./install.sh /dev/cu.usbserial-0001 WIFI   # or BLE (default)
```

## Configuration

### Secrets

On the first run `install.sh` creates `secrets.h` from
`secrets.h.example`; fill in the Wi-Fi credentials, the API key (min. 8
characters, must match the key configured in the Sensor Tester app) and
the hostname, then re-run.

## Testing

Wi-Fi:

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

The current content arrives immediately; write the tag with a phone and
a new `{"kind": ...}` message appears within a second. Over BLE, use
nRF Connect: write the API key to the auth characteristic, subscribe to
the data characteristic, and rewrite the tag.

## See also

- `../../python/nfctag/` — the same node for Raspberry Pi & co.
