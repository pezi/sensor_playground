# SCD30 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a SCD30 I2C sensor (temperature, humidity, CO2). The Sensor
Playground app discovers this node via UDP broadcast (port 9133) and polls
it for data over HTTPS (port 9132, `X-Api-Key` header).

It is the Go counterpart of [`../../python/scd30/`](../../python/scd30/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The driver is a port of the [scd30_i2c](https://github.com/RequestForCoffee/scd30)
Python driver (the library the Python node uses): continuous measurement
at a 2 s interval, data-ready polling, Sensirion CRC-8 on every word,
and the readings decoded from big-endian word pairs into IEEE-754
floats — readings match the Python node.

## Wiring (I2C, address 0x61)

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
go build -o sensor_node_scd30 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_scd30 .   # Pi 3/4/5 (64-bit OS)
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
./sensor_node_scd30
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"SCD30","host":"raspberrypi","temperature":22.4,"humidity":45.1,"co2":612.3}
```

`go test` checks the Sensirion CRC-8 (datasheet vector `0xBE 0xEF` →
`0x92`), the per-word CRC frame parsing and the float decoding against
the datasheet example measurement frame (439.09 ppm / 27.2 °C /
48.8 %RH).

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-scd30-go.service`.

## Deviations from the Python node

- None intended — like the Python node, a poll that lands between two
  measurements (2 s interval, data not ready) or hits a CRC mismatch is
  a failed read (REST 503); the app simply polls again.
