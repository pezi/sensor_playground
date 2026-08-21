# SSD1306 OLED Display Node for Sensor Playground (Rust)

This Rust program implements the **display** variant of the Sensor Playground
sensor interface on single-board computers (Raspberry Pi & co.) with an
**SSD1306 128x64 I2C OLED**. Unlike sensor nodes this node consumes data:
the app pushes one JSON command per action over the WebSocket (port 9132,
`X-Api-Key` handshake header) and the node draws it on the panel. It is
discovered via UDP broadcast on port 9133 like every other node.

It is the Rust counterpart of [`../../python/ssd1306/`](../../python/ssd1306/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).

## Protocol

```
{"id": 1, "image": "<base64>"}   show a bitmap (1024 bytes)
{"id": 2, "clear": true}         blank the display
```

The node acknowledges each applied command with the matching id
(`{"id":1,"ok":true}`), or returns an error ACK
(`{"id":1,"ok":false,"error":"..."}`) when validation or the display write
fails. The reply goes to the client that sent the command.

Over BLE the same two actions arrive as binary writes to the command
characteristic instead of as JSON, because one ATT write carries at most
MTU-3 bytes and a frame is 1024 of them:

```
0x01 <offset:u16 big-endian> <bytes...>   stage a chunk
0x02                                      draw the staged frame
0x03                                      blank the display
```

Chunks carry an absolute offset and must arrive contiguously; writing
offset 0 starts a new frame, so a client that reconnects or gives up
mid-frame simply restarts and never has to be told to reset. A `show` with
an incomplete frame is refused rather than drawn as a band of stale pixels.

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

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace. On a non-Pi development host, build from this folder
normally:

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key
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

Set `"emulation": true` in `config.json` to run the node without a panel —
it reports the share of lit pixels for each frame instead of drawing it,
and the WebSocket protocol behaves exactly as it does with hardware. Works
on any machine (macOS/Windows included). The Python node has no emulation
mode; this port adds one so the protocol can be exercised without a
display.

## Usage

```bash
../target/release/sensor_node_ssd1306        # Wi-Fi
sudo ../target/release/sensor_node_ssd1306   # BLE (needs root, see above)
```

No TLS certificates: the WebSocket transport is plain `ws://`, authorized
with the `X-Api-Key` handshake header. The panel is blanked on Ctrl-C
rather than left with a stale image burning.

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

`cargo test` checks the bitmap transposition against frames produced by the
reference Python implementation (a full pseudo-random frame, checked at both
ends and by byte sum), the chunked BLE frame assembler (contiguity, restart
at offset 0, partial frames), the base64 decoder, and the JSON command ACKs
and NACKs.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-ssd1306-rust.service`.
