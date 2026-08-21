# AHT10/AHT20 Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with an ASAIR AHT10 or AHT20 I2C sensor (temperature, humidity). The Sensor
Playground app discovers this node via UDP broadcast (port 9133) and polls
it for data over HTTPS (port 9132, `X-Api-Key` header).

It is the Rust counterpart of [`../../python/aht20/`](../../python/aht20/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).

Both chips share the fixed I2C address `0x38` and the same measurement
protocol; only the calibrate opcode differs, and the driver tries both.
Set `sensor_name` in `config.json` to `AHT10` if that is the chip on
your board so the app shows the right name.

## Wiring (I2C, address 0x38)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

> Note the AHT10 does not tolerate other devices on the same I2C bus;
> give it a dedicated bus or use an AHT20.

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

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
../target/release/sensor_node_aht20
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"AHT20","host":"raspberrypi","temperature":22.4,"humidity":45.1}
```

`cargo test` checks the raw-to-value conversion (20-bit humidity/temperature
split and busy-status handling) against golden values from the Python
node's formulas.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-aht20-rust.service`.

## Deviations from the Python Node

- None — protocol bytes, timing, payloads, emulation and both transports
  match the Python node.
