# VL53L0X Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a VL53L0X I2C sensor (distance (time of flight)).
It is a *push* node: instead of serving readings over REST it pushes
one JSON message per event over a **WebSocket** (`ws://`, port 9132,
`X-Api-Key` checked on the handshake). The app discovers it via UDP
broadcast on port 9133:

| Direction | Message | Meaning |
|-----------|---------|---------|
| node → app | `{"distance": 123}` | distance in mm (on change and as a 1 s heartbeat) |
| node → app | `{"distance": null}` | no target in range |

It is the Go counterpart of [`../../python/vl53l0x/`](../../python/vl53l0x/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The driver is a port of the
[adafruit_vl53l0x](https://github.com/adafruit/Adafruit_CircuitPython_VL53L0X)
CircuitPython driver (the library the Python node uses): the same init
register sequence (reference SPAD selection, tuning settings, timing
budget, ref calibration) and single-shot ranging reads — readings match
the Python node.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

## Wiring (I2C, address 0x29)

| Pi Pin     | Sensor Pin |
|------------|------------|
| 3.3V (Pin 1) | VCC     |
| GND (Pin 6)  | GND     |
| GPIO 2 (Pin 3) | SDA   |
| GPIO 3 (Pin 5) | SCL   |

Enable I2C: `sudo raspi-config` → Interface Options → I2C.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_vl53l0x .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_vl53l0x .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then pushes plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
./sensor_node_vl53l0x
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

`go test` checks the timeout encoding, timing-budget math and reference
SPAD map masking against golden values generated with the
adafruit_vl53l0x reference driver.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-vl53l0x-go.service`.

## Deviations from the Python node

- BLE is not supported (see above); use the Python or Rust node for BLE.
- Everything else — config schema, publish policy (100 ms measure
  interval, 3 mm change threshold, 1 s heartbeat), payloads and the
  driver's register sequences — matches the Python node.
