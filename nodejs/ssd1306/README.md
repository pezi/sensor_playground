# SSD1306 OLED Display Node for Sensor Playground (Node.js)

This Node.js program implements the **display** variant of the Sensor Playground
sensor interface on single-board computers (Raspberry Pi & co.) with an
**SSD1306 128x64 I2C OLED**. Unlike sensor nodes this node consumes data:
the app pushes one JSON command per action over the WebSocket (port 9132,
`X-Api-Key` handshake header) and the node draws it on the panel. It is
discovered via UDP broadcast on port 9133 like every other node.

It is the Node.js counterpart of [`../../python/ssd1306/`](../../python/ssd1306/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to the WebSocket with a warning (use the Python node for BLE).

## Protocol

```
{"id": 1, "image": "<base64>"}   show a bitmap (1024 bytes)
{"id": 2, "clear": true}         blank the display
```

The node acknowledges each applied command with the matching id
(`{"id":1,"ok":true}`), or returns an error ACK
(`{"id":1,"ok":false,"error":"..."}`) when validation or the display write
fails. The reply goes to the client that sent the command.

## Bitmap format

128x64 pixels, 1 bit per pixel, packed horizontally row by row — 16 bytes
per row, MSB of each byte is the leftmost pixel (the
https://javl.github.io/image2cpp/ "horizontal" byte orientation, matching
the dart_periphery SSD1306 example). The node transposes this into the
SSD1306 native page format — 8 pages of 128 column bytes, each byte holding
8 *vertical* pixels with the topmost in the LSB — before writing it over
I2C.

## Wiring (I2C, address 0x3C)

| Pi Pin | Display Pin |
|--------|-------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C. Other boards:
set `i2c_bus` (Raspberry Pi `1`, NanoPi `0`, Banana Pi `2`). Panels
strapped to the alternate address use `"i2c_address": "0x3D"`.

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without a panel —
it reports the share of lit pixels for each frame instead of drawing it,
and the WebSocket protocol behaves exactly as it does with hardware. Works
on any machine (macOS/Windows included). The Python node has no emulation
mode; this port adds one so the protocol can be exercised without a
display.

## Usage

```bash
node sensor_node.js
```

No TLS certificates: the WebSocket transport is plain `ws://`, authorized
with the `X-Api-Key` handshake header. The panel is blanked on Ctrl-C
rather than left with a stale image burning.

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

`npm test` checks the bitmap transposition against frames produced by the
reference Python implementation (including a full pseudo-random frame by
hash) and the JSON command ACKs and NACKs.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-ssd1306-node.service`.
