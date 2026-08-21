# PAJ7620 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a PAJ7620 I2C sensor
(hand gestures). It is a *push* node: instead of serving readings over
REST it pushes one JSON message per event over a **WebSocket** (`ws://`,
port 9132, `X-Api-Key` checked on the handshake). The app discovers it
via UDP broadcast on port 9133.

| Direction | Message | Meaning |
|-----------|---------|---------|
| node → app | `{"gesture": "forward"}` | one message per detected gesture |

The gesture strings match the app's Gesture enum names: `forward`,
`backward`, `right`, `left`, `up`, `down`, `clockwise`, `antiClockwise`,
`wave`. Like the grove.py driver the Python node uses, the nine basic
gestures are reported; the combined gestures (`forwardBackward`,
`rightLeft`, ...) of the ESP32 sketch are not detected.

It is the Go counterpart of [`../../python/paj7620/`](../../python/paj7620/)
and speaks the identical wire protocol. The driver logic is ported from
Seeed's [grove.py](https://github.com/Seeed-Studio/grove.py)
`grove_gesture_sensor.py` — the library the Python node uses.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

### Wiring (I2C)

| Pi Pin     | Sensor Pin |
|------------|------------|
| 3.3V (Pin 1) | VCC     |
| GND (Pin 6)  | GND     |
| GPIO 2 (Pin 3) | SDA   |
| GPIO 3 (Pin 5) | SCL   |

Enable I2C on the Pi:

```bash
sudo raspi-config   # Interface Options > I2C > Enable
```

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_paj7620 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_paj7620 .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
./sensor_node_paj7620
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# gesture pushes, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Wave a hand over the sensor; each gesture prints as a JSON line
(`{"gesture": "forward"}`).

The driver's gesture-code mapping and flag decoding are unit tested:

```bash
go test .
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-paj7620-go.service`.

## Deviations from the Python node

- BLE is not supported (see above).
- grove.py writes each init register with an SMBus *word* write, whose
  trailing 0x00 lands in the following register via address
  auto-increment; this port uses plain single-byte register writes like
  Seeed's Arduino reference driver. Every register the init table sets
  ends up with the same value.
- An I2C read error while polling terminates the node with a
  `Sensor read failed` message (the Python node dies on the same error
  with a traceback).
