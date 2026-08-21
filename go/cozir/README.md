# CozIR CO2 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a CozIR-A sensor (temperature, humidity, CO2). Unlike the other
environment sensors the CozIR is not an I2C device: it talks a simple
ASCII command protocol over a 9600-baud UART. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Go counterpart of [`../../python/cozir/`](../../python/cozir/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

Protocol: `M 4164` selects the humidity/temperature/CO2 output fields,
`K 2` switches to polling mode, and each `Q` returns one measurement line
(`H 00495 T 01234 Z 06399` → 49.5 %RH, 23.4 °C, 639.9 ppm). The port is
driven directly (raw termios) — no serial-port library needed.

## Wiring (UART)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 14 / TXD (Pin 8) | Rx |
| GPIO 15 / RXD (Pin 10) | Tx |

The CozIR is a 3.3 V device — do **not** connect it to 5 V. Enable the
serial port and disable the login shell on it: `sudo raspi-config` →
Interface Options → Serial Port (login shell **No**, hardware **Yes**).
Other boards: set `serial_port` (default `/dev/serial0`).

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_cozir .
cp config.example.json config.json    # edit: api_key, serial_port
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_cozir .   # Pi 3/4/5 (64-bit OS)
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
./sensor_node_cozir
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"COZIR","host":"raspberrypi","temperature":22.1,"humidity":45.2,"co2":612.4}
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-cozir-go.service`.
