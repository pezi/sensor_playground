# Air530 GPS Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a Grove GPS (Air530)
module (latitude, longitude, altitude, satellites in use). Like the
CozIR, the Air530 is not an I2C device: it continuously streams standard
NMEA-0183 sentences over a 9600-baud UART. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Node.js counterpart of [`../../python/air530/`](../../python/air530/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The parser is a port of the dart_periphery `NmeaParser`
([serial_air530.dart](https://github.com/pezi/dart_periphery/blob/main/example/serial_air530.dart)):

| Sentence | Fields used |
|----------|-------------|
| `GGA` (preferred) | Latitude, longitude, MSL altitude, satellites in use |
| `GLL` (fallback) | Latitude, longitude |

Sentences with bad checksums are skipped. Until the module has a
position fix (cold start can take ~30 s with sky view) the node answers
`200` with a metadata-only body (`sensor` and `host`, no coordinates),
just like the Python node and the ESP32 sketch; the app shows a
"waiting for satellite fix" screen rather than an error. The port is
configured with `stty(1)` and read as a plain file — no native
serial-port module needed.

## Supported Platforms

| Board | Serial port (`serial_port` in config) |
|-------|---------------------------------------|
| Raspberry Pi | `/dev/serial0` (default) |
| NanoPi / Banana Pi (Armbian) | e.g. `/dev/ttyS1` |

## Wiring (UART)

| Pi Pin | Module Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 14 / TXD (Pin 8) | Rx |
| GPIO 15 / RXD (Pin 10) | Tx |

Enable the serial port and disable the login shell on it: `sudo
raspi-config` → Interface Options → Serial Port (login shell **No**,
hardware **Yes**). Other boards: set `serial_port` (default
`/dev/serial0`).

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, serial_port
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
node sensor_node.js
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"AIR530","host":"raspberrypi","latitude":50.085398,"longitude":18.198643,"altitude":202.8,"satellites":5}
```

The NMEA parser has unit tests (golden values from the Python node's
parser): `node test_driver.js`

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-air530-node.service`.

## Deviations from the Python node

- BLE is not supported; `"transport": "ble"` falls back to Wi-Fi with a
  warning.
- The shared Node.js transport turns a `null` reading into a 503, so the
  fixless warm-up state is replicated by returning an **empty** reading
  object — the REST response is then the same 200 metadata-only body the
  Python node produces via `run_rest_server(..., allow_empty=True)`.
- The serial burst is collected from a background read stream with a
  50 ms wake-up granularity instead of a blocking `serial.read(512)`;
  the resulting ~1 s, ≤ 512-byte burst is the same.
