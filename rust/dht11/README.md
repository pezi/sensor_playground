# DHT11 Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a DHT11 sensor
(temperature, humidity) — the blue
[Grove Temperature & Humidity Sensor](https://wiki.seeedstudio.com/Grove-TemperatureAndHumidity_Sensor/)
module. The Sensor Playground app discovers this node via UDP broadcast
(port 9133) and polls it for data over HTTPS (port 9132, `X-Api-Key`
header).

It is the Rust counterpart of [`../../python/dht11/`](../../python/dht11/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).

The DHT22 — the white Grove "Pro" module — has a dedicated Rust node in
[`../dht22/`](../dht22/). Older configurations may still set
`sensor_name` here to `DHT22`; the matching hardware decoding remains
supported for compatibility.

> **Single-wire timing.** The DHT protocol is timing-critical (26–70 µs
> pulses), so the pin is read via kernel-timestamped GPIO edge events
> (character device, `/dev/gpiochipN`, the `gpiocdev` crate): the node
> drives the line low for 18 ms, releases it, and decodes the sensor's
> answer from the intervals between falling edges (~76–78 µs = 0,
> ~120 µs = 1). A userspace polling loop would drown that difference in
> jitter. There is no extension-hat option: the Arduino-based hats are
> polled over I2C and cannot follow those pulses either. Reads
> occasionally fail even on a healthy sensor; the node retries and
> serves the last good reading for up to 30 seconds, so a single failed
> read does not surface as an error.

## Supported Platforms

| Board | Notes |
|-------|-------|
| Raspberry Pi | `gpio_chip: 0`; the signal pin is a plain GPIO |

### Wiring (single-wire digital)

| Pi Pin | Grove Pin |
|--------|-----------|
| 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 4 (Pin 7, `gpio_pin` in config) | SIG (yellow) |

> The Grove module already carries the required pull-up resistor on the
> signal line; add a 10 kΩ pull-up to 3.3V when wiring a bare DHT11.

## Setup

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace. On a non-Pi development host, build from this folder
normally:

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key, gpio_pin
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
hardware — it then serves plausible generated readings quantised to the
DHT11's whole-degree resolution. Works on any machine (macOS/Windows
included).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
../target/release/sensor_node_dht11
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"DHT11","host":"raspberrypi","temperature":22.0,"humidity":46.0}
```

`cargo test -p sensor-playground-dht11` checks the falling-edge bit-train
decoder and the checksum against synthetic timestamp trains, in both the
DHT11 and the DHT22 payload format (including negative DHT22
temperatures).

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-dht11-rust.service`.

## Deviations from the Python Node

- The single-wire read decodes GPIO character-device edge events instead
  of using the compiled pulse reader bundled with
  `adafruit-circuitpython-dht`. The line is requested once and
  reconfigured per read (output for the 18 ms start signal, then
  falling-edge input for the answer), so no second request has to race
  the first data edge.
- `gpio_chip` selects the GPIO character device (`/dev/gpiochipN`,
  Raspberry Pi `0`); the Python config has no such key. Everything else —
  payloads, config schema, BLE transport, throttle/cache policy,
  emulation — matches the Python node.
