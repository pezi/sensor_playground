# SHT11 Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a Sensirion SHT1x sensor
(temperature, humidity) — the classic SHT10 / SHT11 / SHT15 family. The
Sensor Playground app discovers this node via UDP broadcast (port 9133)
and polls it for data over HTTPS (port 9132, `X-Api-Key` header).

It is the Rust counterpart of [`../../python/sht11/`](../../python/sht11/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).

The chips of the family differ only in calibration accuracy and speak the
same protocol; set `sensor_name` in `config.json` to the one on your board
so the app shows the right name.

> **Not I2C.** The SHT1x uses a proprietary two-wire protocol (SCK plus a
> bidirectional DATA line) that resembles I2C but is not compatible with
> it — the sensor cannot share an I2C bus. The bus is fully master-clocked
> with no minimum speed, so it is bit-banged over the GPIO character
> device (`/dev/gpiochipN`, the `gpiocdev` crate): one call per clock edge
> is harmless because the sensor simply waits between edges. DATA is
> driven open-drain style — released (input with pull-up) for a 1, pulled
> LOW for a 0 — since the sensor drives the same wire when answering.

> **Self-heating.** Measuring more than ~10 % of the time warms the chip
> and biases the reading, so the node performs at most one hardware read
> every two seconds and serves the cached values in between. A failed
> transfer resynchronises the bus and keeps serving the cache; only after
> 30 seconds without a successful read does the REST endpoint answer 503.

## Supported Platforms

| Board | GPIO chip (`gpio_chip` in config) |
|-------|-----------------------------------|
| Raspberry Pi | `0` (default, `/dev/gpiochip0`) |
| Other boards | the chip carrying the header pins |

### Wiring (two-wire, NOT I2C)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VDD |
| GND (Pin 6) | GND |
| GPIO 4 (Pin 7, `data_pin` in config) | DATA |
| GPIO 5 (Pin 29, `sck_pin` in config) | SCK |

> The DATA line needs a pull-up resistor (~10 kΩ to 3.3V); most breakout
> boards carry one, and the node additionally enables the Pi's internal
> pull-up. The temperature conversion constant assumes a 3.3V supply.

## Setup

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace; build from this folder (on a Raspberry Pi consider
cross-compiling instead — see the BME680 README):

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key, data_pin, sck_pin
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
../target/release/sensor_node_sht11        # Wi-Fi
sudo ../target/release/sensor_node_sht11   # BLE (needs root, see above)
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"SHT11","host":"raspberrypi","temperature":22.4,"humidity":45.1}
```

`cargo test` checks the datasheet conversion formulas (14-bit temperature,
12-bit humidity polynomial with temperature compensation) at exact raw
values, including the 0–100 %RH clamp.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-sht11-rust.service`.

## Deviations from the Python node

- `gpio_chip` selects `/dev/gpiochipN`; the Python node uses lgpio's chip
  handle for the same purpose.
- The DATA line is requested once and *reconfigured* in place for each
  direction flip, where the Python node re-claims it per flip. The wire
  behaviour is identical.
- Like the Python node, the transfer ends before the sensor's CRC byte, so
  no checksum is validated.
