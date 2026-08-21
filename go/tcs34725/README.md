# TCS34725 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a **TCS34725** I2C sensor
(RGB colour, colour temperature, illuminance). The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Go counterpart of [`../../python/tcs34725/`](../../python/tcs34725/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python node for BLE).

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

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_tcs34725 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_tcs34725 .   # Pi 3/4/5 (64-bit OS)
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
./sensor_node_tcs34725
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

`go test` checks the gamma-corrected RGB bytes and the DN40 lux/colour
temperature maths against values produced by the reference Python
implementation, including the saturation cut-off.

## Deviations from the Python node

- One conversion per request. The Python node reads the sensor once per
  property — three times per request — so its colour, illuminance and
  colour temperature can come from three different integrations; this port
  derives all three from a single one.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-tcs34725-go.service`.
