# DHT11 Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface on
single-board computers with a DHT11 sensor (temperature, humidity) — the
blue
[Grove Temperature & Humidity Sensor](https://wiki.seeedstudio.com/Grove-TemperatureAndHumidity_Sensor/)
module. The Sensor Playground app discovers this node via UDP broadcast
(port 9133) and polls it for data over HTTPS (port 9132, `X-Api-Key`
header).

It is the Node.js counterpart of
[`../../python/dht11/`](../../python/dht11/) and speaks the identical
wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The DHT22 — the white Grove "Pro" module — has a dedicated Node.js node
in [`../dht22/`](../dht22/) with one-decimal DHT22-style emulation.

> **Emulation only.** The DHT single-wire protocol is timing-critical: a
> bit is a 26–28 µs (0) or 70 µs (1) high pulse, so the frame has to be
> decoded from kernel edge timestamps — no Node.js GPIO path delivers
> those timestamps (a JavaScript polling loop adds milliseconds of
> jitter, thousands of times the pulse difference). There is no
> extension-hat option either: the Arduino-based hats are polled over
> I2C and cannot follow those pulses. With `"emulation": false` the node
> prints an error and exits; use the Python, Go or Rust node for the
> real sensor.

## How the real sensor is read

The Python, Go and Rust nodes drive the signal line low for 18 ms (the
start signal), release it, and decode the sensor's answer from the
intervals between falling edges:

    ~76-78 µs falling-to-falling  ->  bit 0
    ~120 µs  falling-to-falling   ->  bit 1

The 40 bits carry five bytes — humidity, temperature and a checksum —
decoded as whole values (DHT11) or 16-bit tenths with a sign bit
(DHT22). Reads occasionally fail even on a healthy sensor, so those
nodes retry every 2 s and serve the last good reading for up to 30
seconds. This port only emulates the resulting `temperature` /
`humidity` values.

## Supported Platforms

| Board | Notes |
|-------|-------|
| any machine | emulation only (macOS/Windows/Linux) |

### Wiring (single-wire digital, for the Python/Go/Rust nodes)

| Pi Pin | Grove Pin |
|--------|-----------|
| 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 4 (Pin 7, `gpio_pin` in config) | SIG (yellow) |

> The Grove module already carries the required pull-up resistor on the
> signal line; add a 10 kΩ pull-up to 3.3V when wiring a bare DHT11.

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, set "emulation": true
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
sensor hardware — it then serves plausible generated readings quantised
to the DHT11's whole-degree resolution. Works on any machine
(macOS/Windows included). This is the only mode this port supports (see
above).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
node sensor_node.js
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"DHT11","host":"raspberrypi","temperature":22,"humidity":46}
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-dht11-node.service`.

## Deviations from the Python Node

- **Real hardware is not supported** — this port runs in emulation mode
  only. Decoding the DHT single-wire frame needs kernel-timestamped GPIO
  edge events, which Node.js cannot access (see above); the Python, Go
  and Rust nodes drive the real sensor. `gpio_pin` is therefore accepted
  in `config.json` (so the same file works for all ports) but unused,
  and the 2 s retry / 30 s cache policy has nothing to cache: the
  emulated reading never fails, so the node never answers 503
  `{"error":"sensor_read_failed"}`.
- BLE transport falls back to Wi-Fi (see above).
- `sensor_name` still selects the reported name for older configurations,
  but this node always uses DHT11 whole-value emulation. Use `../dht22/`
  for DHT22 one-decimal emulation. The payload keys and config schema
  match the Python node.
