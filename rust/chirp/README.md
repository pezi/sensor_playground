# Chirp I2C Soil Moisture Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a **Catnip Electronics
I2C Soil Moisture Sensor** (the "Chirp" sensor, default I2C address
`0x20`). It reports the soil moisture as a percentage (JSON key
`moisture`, mapped linearly between the two capacitance calibration
points in `config.json`), the soil temperature (`temperature`) and the
ambient light level (`light`), alongside the raw capacitance (`cap`) for
calibrating. The Sensor Playground app discovers this node via UDP
broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header).

It is the Rust counterpart of [`../../python/chirp/`](../../python/chirp/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).

The driver is a port of the register access in the Python node (smbus2),
following the same register map as the Arduino reference library
([I2CSoilMoistureSensor](https://github.com/Apollon77/I2CSoilMoistureSensor)):
a reset at startup, big-endian 16-bit reads of the capacitance,
temperature and light registers, and the light measurement started by
writing register `0x03`.

The sensor is capacitive: no exposed metal electrodes, so it can stay in
soil permanently without corroding. Unlike the analog Grove probe it
talks plain I2C — **no ADC hat needed**.

## Readings & calibration

| JSON key (REST) | Meaning |
|-----------------|---------|
| `moisture` | soil moisture, `0`-`100` %, mapped between `cap_dry`/`cap_wet` |
| `temperature` | soil/sensor temperature in °C (chip reports tenths) |
| `light` | ambient brightness in raw counts, **higher = brighter** (the chip counts a phototransistor discharge upward in darkness; the node inverts it, `65535 - raw`) |
| `cap` | the raw capacitance, for calibrating the two points below |

The raw capacitance rises with moisture, so **wet > dry**:

| Key | Meaning | Default |
|-----|---------|---------|
| `cap_dry` | raw capacitance in dry air | `290` |
| `cap_wet` | raw capacitance submerged in water | `520` |

Individual sensors vary, so **calibrate yours**: read `cap` from the REST
payload with the sensor in dry air and again submerged up to the marked
line in a glass of water, and put the two numbers into `config.json`.
Values outside the calibrated span clamp to 0/100 %.

A light measurement takes up to three seconds on the chip, so the value
is harvested from a measurement started on an earlier poll; the `light`
key is absent until the first one completes (a few seconds after start).

## Wiring

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 / SDA (Pin 3) | SDA |
| GPIO 3 / SCL (Pin 5) | SCK/SCL |

Only the coated probe part goes into the soil — keep the components at
the top dry.

Enable I2C: `sudo raspi-config` → Interface Options → I2C. Other boards:
set `i2c_bus` (Raspberry Pi `1`, NanoPi `0`, Banana Pi `2`). The
`address` is decimal in `config.json` (`32` = `0x20`).

## Setup

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace. On a non-Pi development host, build from this folder
normally:

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key, cap_dry, cap_wet
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
hardware — it then serves a plausible watering cycle with temperature and
a day/night light curve (works with both transports). Works on any
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
../target/release/sensor_node_chirp
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"cap":400,"host":"raspberrypi","light":52000,"moisture":48,"sensor":"CHIRP","temperature":21.3}
```

`cargo test -p sensor-playground-chirp` checks the calibration mapping
(both clamps), the big-endian register words, the signed temperature
decoding (including negative values) and the light inversion.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-chirp-rust.service`.

## Deviations from the Python node

- The Python node reads a register with one combined SMBus transaction
  (smbus2's `read_i2c_block_data`). This port writes the register
  address and reads the two bytes as separate transactions with the
  20 ms pause of the Arduino reference library in between, which is what
  the chip expects for that access pattern.
- The REST and discovery paths share one sensor behind a mutex, so the
  light state machine and the I2C access are serialized (Python serves
  both from threads without a lock).
