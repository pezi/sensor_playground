# PAJ7620 Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a PAJ7620 I2C sensor
(hand gestures). It is a *push* node: instead of serving readings over
REST it pushes one JSON message per event over a **WebSocket** (`ws://`,
port 9132, `X-Api-Key` checked on the handshake). The app discovers it
via UDP broadcast on port 9133:

| Direction | Message | Meaning |
|-----------|---------|---------|
| node → app | `{"gesture": "forward"}` | one message per detected gesture |

The gesture strings match the app's Gesture enum names: `forward`,
`backward`, `right`, `left`, `up`, `down`, `clockwise`, `antiClockwise`,
`wave`. Like the grove.py driver the Python node uses, the nine basic
gestures are reported; the combined gestures (`forwardBackward`,
`rightLeft`, ...) of the ESP32 sketch are not detected.

It is the Rust counterpart of
[`../../python/paj7620/`](../../python/paj7620/) and speaks the
identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).
  Over BLE each gesture arrives as a notify on the data characteristic;
  the node takes no commands.

The driver is a port of Seeed's
[grove.py](https://github.com/Seeed-Studio/grove.py)
`grove_gesture_sensor.py` (the library the Python node uses): the same
wake-up retry, the same init register table (both banks) and the same
two-phase flag decoding with the 0.8 s entry / 1.0 s quit waits around
the combined forward/backward gestures.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

## Wiring (I2C, address 0x73)

| Pi Pin     | Sensor Pin |
|------------|------------|
| 3.3V (Pin 1) | VCC     |
| GND (Pin 6)  | GND     |
| GPIO 2 (Pin 3) | SDA   |
| GPIO 3 (Pin 5) | SCL   |

Enable I2C: `sudo raspi-config` → Interface Options → I2C.

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

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then pushes plausible generated readings (one random
gesture every three to five seconds). Works on any machine
(macOS/Windows included).

## Usage

```bash
../target/release/sensor_node_paj7620
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Wave a hand over the sensor; each gesture arrives as a JSON line
(`{"gesture": "forward"}`).

`cargo test` checks the driver's gesture-code mapping against the Python
node's `GESTURES` dict and grove.py's two-phase flag decoding (including
the entry/quit waits) with reads and sleeps injected.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-paj7620-rust.service`.

## Deviations from the Python node

- grove.py writes each init register with an SMBus *word* write, whose
  trailing 0x00 lands in the following register via address
  auto-increment; this port uses plain single-byte register writes like
  Seeed's Arduino reference driver. Every register the init table sets
  ends up with the same value.
- An I2C read error while polling terminates the node with a
  `Sensor read failed` message (the Python node dies on the same error
  with a traceback).
- Everything else — config schema, 100 ms poll interval, both
  transports, the emulation behavior and the payload — matches the
  Python node.
