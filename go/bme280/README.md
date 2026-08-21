# BME280 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a BME280 I2C sensor (temperature, humidity, pressure). The Sensor
Playground app discovers this node via UDP broadcast (port 9133) and polls
it for data over HTTPS (port 9132, `X-Api-Key` header).

It is the Go counterpart of [`../../python/bme280/`](../../python/bme280/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The driver is a port of the [RPi.bme280](https://github.com/rm-hull/bme280)
Python driver (the library the Python node uses): double-precision
compensation formulas, forced mode, x1 oversampling — readings match the
Python node.

## Wiring (I2C, address 0x76)

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
go build -o sensor_node_bme280 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_bme280 .   # Pi 3/4/5 (64-bit OS)
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
./sensor_node_bme280
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"BME280","host":"raspberrypi","temperature":22.4,"humidity":45.1,"pressure":1013.2}
```

`go test` checks the compensation math against 200 golden values
generated with the RPi.bme280 reference driver.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-bme280-go.service`.
