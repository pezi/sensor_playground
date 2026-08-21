# SI1145 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a **SI1145** I2C sensor
(visible light, infrared, UV index) — for example the Grove Sunlight
Sensor. The Sensor Playground app discovers this node via UDP broadcast
(port 9133) and polls it for data over HTTPS (port 9132, `X-Api-Key`
header).

It is the Go counterpart of [`../../python/si1145/`](../../python/si1145/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python node for BLE).

The driver is a port of the `SI1145` PyPI package the Python node uses
(itself a port of Adafruit's Arduino library): the same reset sequence, UV
calibration coefficients, channel list and ADC settings, and the same
autonomous measurement mode — so the counts match the Python node. Unlike
the Python library it checks the part ID (`0x45`) first, which turns a
missing or mis-wired chip into a clear error instead of nonsense counts.

## Readings

| JSON key (REST) | Meaning |
|-----------------|---------|
| `visible` | raw visible-light counts (the SI1145 is **not** lux-calibrated) |
| `ir` | raw infrared counts |
| `uv` | UV index, computed by the chip from the visible/IR photodiodes |

Visible and IR sit at a dark baseline of roughly 250-260 counts rather than
0 — that is the chip's offset, not a fault. The UDP discovery reply carries
the same three values as `vis`/`ir`/`uv`.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

### Wiring (I2C, address 0x60)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_si1145 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_si1145 .   # Pi 3/4/5 (64-bit OS)
```

Unlike the Python node this port needs no build toolchain and no
`Adafruit-GPIO`/`spidev` C extensions — the driver is built in.

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
./sensor_node_si1145
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"SI1145","host":"raspberrypi","visible":274,"ir":259,"uv":0.1}
```

`go test` checks the UV index scaling (the chip reports it ×100) and the
one-decimal rounding of the payload.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-si1145-go.service`.
