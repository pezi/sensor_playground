# TSL2591 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a **TSL2591** I2C sensor
(visible light, infrared, illuminance). The Sensor Playground app discovers
this node via UDP broadcast (port 9133) and polls it for data over HTTPS
(port 9132, `X-Api-Key` header).

It is the Go counterpart of [`../../python/tsl2591/`](../../python/tsl2591/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python node for BLE).

The driver is a port of the `adafruit_tsl2591` library the Python node
uses: the same device-ID check, the same gain and integration-time
encoding, and the same two-equation lux formula — so the values match the
Python node.

## Readings

The chip has two photodiodes: channel 0 is broadband (visible + IR) and
channel 1 is infrared only. Neither is "visible light" on its own — the
difference of the two is. Lux is a third thing again, derived from both
channels together with the gain and integration time.

| JSON key (REST) | Meaning |
|-----------------|---------|
| `visible` | broadband minus infrared counts, floored at 0 |
| `ir` | infrared counts |
| `lux` | illuminance; **absent** when a channel saturated |

The UDP discovery reply carries the same values as `vis`/`ir`/`lux`.

## Gain and integration time

| Config key | Default | Values |
|------------|---------|--------|
| `gain` | `"med"` | `low` (1x), `med` (25x), `high` (428x), `max` (9876x) |
| `integration_ms` | `300` | `100`, `200`, `300`, `400`, `500`, `600` |
| `auto_gain` | `true` | move one gain step when the broadband channel pins |

Auto-gain moves a single step per reading: changing the gain invalidates
the integration already in flight, so the *next* reading is the one that
benefits. Jumping straight to the extreme instead would make the value
oscillate whenever the light sits near a threshold.

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
go build -o sensor_node_tsl2591 .
cp config.example.json config.json    # edit: api_key, i2c_bus, gain
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_tsl2591 .   # Pi 3/4/5 (64-bit OS)
```

Unlike the Python node this port needs no Blinka/`adafruit-extended-bus`
stack — the driver is built in and talks to `/dev/i2c-N` directly.

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves a lit room near a window, a few hundred lux
drifting on a slow cycle. Works on any machine (macOS/Windows included).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
./sensor_node_tsl2591
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"TSL2591","host":"raspberrypi","visible":3900,"ir":1500,"lux":159.9}
```

`go test` checks the lux equation against values produced by the reference
Python implementation, including the integration-time-dependent saturation
limit (the ADC only counts to `0x8FFF` at 100 ms).

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-tsl2591-go.service`.
