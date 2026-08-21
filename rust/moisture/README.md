# Grove Capacitive Moisture Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a **Grove Capacitive
Moisture Sensor (Corrosion-Resistant)** — an analog probe whose output
voltage falls as the soil gets wetter. It reports the soil moisture as a
percentage (JSON key `moisture`) mapped linearly between the two
calibration points in `config.json`, alongside the raw reading
(`adc`/`adcMax`) for calibrating them. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Rust counterpart of [`../../python/moisture/`](../../python/moisture/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).

## Needs a Grove Base Hat

The Raspberry Pi has no analog input, so the probe is read through the
Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04): plug the probe
into one of the hat's analog ports (A0-A7) and set `pin` to that channel.
Do not insert the probe deeper than the marked line — the electronics at
the top are not waterproof.
Enable I2C: `sudo raspi-config` → Interface Options → I2C.

> The Arduino-based hats the Python node also supports (`"nano"`,
> `"grovePlus"`) are **not** implemented in this port — use the Python
> node for those.

## Calibration

`moisture` is a `0`-`100` % value mapped linearly between two calibration
points in `config.json`:

| Key | Meaning | Default |
|-----|---------|---------|
| `adc_dry` | averaged raw ADC reading in dry air | `2600` |
| `adc_wet` | averaged raw ADC reading submerged in water | `1100` |

Probes and hats vary, so **calibrate yours**: the REST payload also
carries the averaged raw reading (`adc`, with its full scale `adcMax`).
Hold the probe in dry air, read `adc`, put the number into `adc_dry`;
submerge it up to the marked line in a glass of water and do the same
for `adc_wet`. Values outside the calibrated span clamp to 0/100 %.

## Setup

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace. On a non-Pi development host, build from this folder
normally:

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key, pin, adc_dry, adc_wet
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
hardware — it then serves a plausible watering-and-drying cycle (works
with both transports). Works on any machine (macOS/Windows included).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
../target/release/sensor_node_moisture
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"MOISTURE","host":"raspberrypi","moisture":63,"adc":1820,"adcMax":4095}
```

The calibration math has unit tests:
`cargo test -p sensor-playground-moisture`

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-moisture-rust.service`.
