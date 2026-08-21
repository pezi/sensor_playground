# TCS34725 Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a **TCS34725** I2C sensor
(RGB colour, colour temperature, illuminance). The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Rust counterpart of [`../../python/tcs34725/`](../../python/tcs34725/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).

The driver is a port of the `adafruit_tcs34725` library the Python node
uses: the same sensor-ID check, the same gamma-corrected RGB bytes, and the
same DN40 lux / colour temperature algorithm — so the values match the
Python node.

> **Not the library defaults.** Both nodes program 154 ms integration at 4x
> gain. The library's own defaults (2.4 ms, 1x) collect almost no light:
> every channel reads 0 in normal room light and the readings collapse to
> the same constants (0 lux, ~1391 K).

## Readings

| JSON key (REST) | Meaning |
|-----------------|---------|
| `red`/`green`/`blue` | measured colour, normalized against the clear channel with a 2.5 gamma correction (`0`-`255`) |
| `colorTemperature` | correlated colour temperature in Kelvin (DN40) |
| `lux` | illuminance, IR-rejected (DN40) |

A saturated clear channel says nothing about the colour, so the node
answers 503 rather than reporting a wrong one. The UDP discovery reply
carries `ct` and `lux`.

## Wiring (I2C, address 0x29)

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

Unlike the Python node this port needs no Blinka/`adafruit-extended-bus`
stack — the driver is built in and talks to `/dev/i2c-N` directly.

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves a coloured light cycling through the hue circle
every 30 seconds. Works on any machine (macOS/Windows included).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
../target/release/sensor_node_tcs34725        # Wi-Fi
sudo ../target/release/sensor_node_tcs34725   # BLE (needs root, see above)
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"TCS34725","host":"raspberrypi","colorTemperature":4820,"lux":472,"red":11,"green":17,"blue":8}
```

`cargo test` checks the gamma-corrected RGB bytes and the DN40 lux/colour
temperature maths against values produced by the reference Python
implementation, including the saturation cut-off.

## Deviations from the Python node

- One conversion per request. The Python node reads the sensor once per
  property — three times per request — so its colour, illuminance and
  colour temperature can come from three different integrations; this port
  derives all three from a single one.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-tcs34725-rust.service`.
