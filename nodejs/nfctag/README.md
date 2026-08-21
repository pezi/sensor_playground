# Grove NFC Tag Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a Grove NFC Tag — a
**passive dual-interface EEPROM** (ST M24LR64E-R, 8 KB). The board is
not a reader: a phone or any ISO 15693 NFC writer stores an NDEF
message on it over RF, and this node reads the same memory over I2C. It
is a *push* node: it polls the EEPROM once a second and pushes one JSON
message over a **WebSocket** (`ws://`, port 9132, `X-Api-Key` checked
on the handshake) whenever the content changes, plus UDP discovery on
port 9133.

It is the Node.js counterpart of [`../../python/nfctag/`](../../python/nfctag/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to the WebSocket with a warning (use the Python or Rust node for BLE).

## Protocol

One message per content change — and, because the tag holds state, the
current content is also sent to every client right after it connects:

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

`i2c-bus` is an *optional* native dependency: if its build fails,
`npm install` still succeeds and emulation mode keeps working; real
tag mode then explains what to install.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
tag hardware — it then cycles through a text, a URI and an empty
content every 15 seconds. Works on any machine (macOS/Windows
included).

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
```

The current content arrives immediately; write the tag with a phone and
a new `{"kind": ...}` message appears within a second.

`npm test` checks the NDEF parser (capability container + TLV stream,
Text/URI records, hex-dump fallback) against the Python node's behavior.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-nfctag-node.service`.

## Deviations from the Python node

- BLE is not supported; `"transport": "ble"` falls back to the
  WebSocket with a warning.
- The EEPROM is read with separate address-write/read transactions
  instead of the Python node's combined `i2c_rdwr` transaction — the
  M24LR64's address pointer survives the stop condition in between, so
  the bytes read are the same.
