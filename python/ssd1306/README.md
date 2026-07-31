# SSD1306 Display Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with an SSD1306 128×64 I2C OLED display.
It is a *display* node: instead of producing readings it **consumes**
data — the app pushes one command per action and the node draws it on
the panel.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_ssd1306/` and supports the same two transports.

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket server (`ws://`, port 9132, `X-Api-Key`
  checked on the handshake) plus UDP discovery on port 9133. No SSL
  certificates: unlike the sensor nodes this one speaks `ws://`, not HTTPS.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketch (see
  `../common/README.md` for the GATT contract, BlueZ prerequisites and
  testing). No UDP discovery; BLE advertising is the discovery. Only one
  BLE node can run per board.

## Protocol

Over **Wi-Fi** a command is one JSON message:

| Command | Effect |
|---------|--------|
| `{"image": "<base64>"}` | Show a bitmap |
| `{"clear": true}` | Blank the display |

Over **BLE** the same two actions arrive as binary writes to the command
characteristic, because one ATT write carries at most MTU-3 bytes and a
frame is 1024 of them:

| Packet | Effect |
|--------|--------|
| `0x01 <offset:u16 big-endian> <bytes…>` | Stage a chunk at `offset` |
| `0x02` | Draw the staged frame |
| `0x03` | Blank the display |

The chunks are sent raw rather than base64-encoded (BLE writes are
binary-safe), which keeps the transfer at 1024 bytes instead of ~1.4 KB.
They must arrive contiguously; writing offset 0 restarts the frame, and a
`0x02` on an incomplete frame is refused rather than drawn. See
`FrameAssembler` in `sensor_node.py`.

The bitmap is 1024 bytes (128×64 pixels, 1 bit per pixel), packed
horizontally row by row — 16 bytes per row, the MSB of each byte is the
leftmost pixel. This matches the
[dart_periphery SSD1306 example](https://github.com/pezi/dart_periphery/blob/main/example/i2c_ssd1306.dart)
and the [image2cpp](https://javl.github.io/image2cpp/) "horizontal" byte
orientation. The node transposes it into the SSD1306 native page format
(8 pages × 128 column bytes, LSB on top) before writing it over I2C.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

Most SSD1306 boards use I2C address `0x3C` (default); some are strapped
to `0x3D` — set `i2c_address` in the config accordingly.

### Wiring (I2C)

| Pi Pin     | Display Pin |
|------------|-------------|
| 3.3V (Pin 1) | VCC       |
| GND (Pin 6)  | GND       |
| GPIO 2 (Pin 3) | SDA     |
| GPIO 3 (Pin 5) | SCL     |

Enable I2C on the Pi:

```bash
sudo raspi-config   # Interface Options > I2C > Enable
```

## Software Setup

```bash
# Create virtual environment
python3 -m venv venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Copy and edit configuration
cp config.example.json config.json
# Edit config.json: set api_key and the i2c_bus for your board
```

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

Wi-Fi, with [`websocat`](https://github.com/vi/websocat):

```bash
echo '{"clear": true}' | \
  websocat -H='X-Api-Key: your-sensor-api-key' ws://localhost:9132/
```

BLE, with a generic client such as nRF Connect: connect, write the API key
to `…0003…` as a UTF-8 string, then write `03` to `…0004…` to blank the
panel. Sending a whole bitmap by hand is impractical — use the app.

## Running as a Service (BLE)

The BLE transport needs root on Raspberry Pi (see `../common/README.md`),
and it depends on Bluetooth rather than the network. In the unit file below
replace the `[Unit]` dependencies with `After=bluetooth.target` /
`Wants=bluetooth.target` and set `User=root`.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-ssd1306.service`:

```ini
[Unit]
Description=Sensor Tester SSD1306 Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-ssd1306
ExecStart=/home/pi/sensor-ssd1306/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-ssd1306
sudo systemctl start sensor-tester-ssd1306
```
