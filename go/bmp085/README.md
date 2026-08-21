# BMP085 Barometer Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a BMP085 I2C barometer — the sensor behind the Grove Barometer Sensor.
Reports temperature, barometric pressure and the altitude derived from it;
the pin-compatible BMP180 works unchanged. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Go counterpart of [`../../python/bmp085/`](../../python/bmp085/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The driver is a direct transcription of the integer compensation
algorithm in the BMP085 datasheet, kept line-for-line in step with the
Python node and the ESP32 sketch.

## Wiring (I2C, fixed address 0x77)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C. `oversampling`
(0-3, default 3) trades conversion time for pressure resolution.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_bmp085 .
cp config.example.json config.json    # edit: api_key, i2c_bus, oversampling
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_bmp085 .   # Pi 3/4/5 (64-bit OS)
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
./sensor_node_bmp085
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"BMP085","host":"raspberrypi","temperature":21.3,"pressure":1013.19,"altitude":0.5}
```

`go test` verifies the compensation against the datasheet's worked
example (15.0 °C / 69964 Pa).

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-bmp085-go.service`.
