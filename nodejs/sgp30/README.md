# SGP30 Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a SGP30 I2C sensor (eCO2, TVOC). The Sensor
Playground app discovers this node via UDP broadcast (port 9133) and polls
it for data over HTTPS (port 9132, `X-Api-Key` header).

It is the Node.js counterpart of [`../../python/sgp30/`](../../python/sgp30/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The driver is a port of the [Pimoroni sgp30-python](https://github.com/pimoroni/sgp30-python)
driver (the library the Python node uses): 16-bit command words, CRC-8
checked response words, and the same blocking warm-up that discards the
sensor's initialization readings — readings match the Python node.

## Wiring (I2C, address 0x58)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C. Other boards:
set `i2c_bus` (Raspberry Pi `1`, NanoPi `0`, Banana Pi `2`).

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, i2c_bus
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

The SGP30 needs a ~15 second warm-up after start; the program blocks until
the sensor stops reporting its fixed initialization values (400 ppm /
0 ppb).

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"SGP30","host":"raspberrypi","eco2":612,"tvoc":58}
```

`npm test` checks the CRC-8 and response parsing against the SGP30
datasheet example and vectors from the Pimoroni reference driver.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-sgp30-node.service`.
