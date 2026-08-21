# MMA7660 Accelerometer Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a
[Grove 3-Axis Digital Accelerometer ±1.5g](https://wiki.seeedstudio.com/Grove-3-Axis_Digital_Accelerometer-1.5g/)
(MMA7660FC). The raw axes are converted into roll and pitch angles plus the
total acceleration magnitude (g-force). The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and then **streams** the
readings from a WebSocket (port 9132, `ws://`, `X-Api-Key` header on the
handshake) — the app streams accelerometers rather than polling them;
readings are pushed every 250 ms.

It is the Rust counterpart of [`../../python/mma7660/`](../../python/mma7660/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).
  Over BLE the readings arrive as notifies on the data characteristic
  at the same 250 ms motion cadence.

The MMA7660 is driven directly over I2C like the Python node's smbus2
access (6-bit two's-complement axes, 21.33 counts per g) — no extra
driver crate needed.

## Wiring (I2C, address 0x4C)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C. Other boards:
set `i2c_bus` (Raspberry Pi `1`, NanoPi `0`, Banana Pi `2`).

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
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
../target/release/sensor_node_mma7660
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# stream readings, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

```json
{"sensor":"MMA7660","host":"raspberrypi","roll":2.1,"pitch":-0.8,"gforce":1.01}
```

`cargo test` checks the 6-bit two's-complement axis decoding, the
counts-per-g conversion and the roll/pitch/g-force math.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-mma7660-rust.service`.

## Deviations from the Python node

- A failed sensor read is logged and that push skipped; the Python node
  lets the exception end the process.
