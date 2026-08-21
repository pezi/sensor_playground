# CozIR CO2 Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a CozIR-A sensor (temperature, humidity, CO2). Unlike the other
environment sensors the CozIR is not an I2C device: it talks a simple
ASCII command protocol over a 9600-baud UART. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Node.js counterpart of [`../../python/cozir/`](../../python/cozir/)
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
{"sensor":"COZIR","host":"raspberrypi","temperature":22.1,"humidity":45.2,"co2":612.4}
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-cozir-node.service`.
