# AHT10/AHT20 Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with an ASAIR AHT10 or AHT20 I2C sensor (temperature, humidity). The Sensor
Playground app discovers this node via UDP broadcast (port 9133) and polls
it for data over HTTPS (port 9132, `X-Api-Key` header).

It is the Node.js counterpart of [`../../python/aht20/`](../../python/aht20/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

Both chips share the fixed I2C address `0x38` and the same measurement
protocol; only the calibrate opcode differs, and the driver tries both.
Set `sensor_name` in `config.json` to `AHT10` if that is the chip on
your board so the app shows the right name.

## Wiring (I2C, address 0x38)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

> Note the AHT10 does not tolerate other devices on the same I2C bus;
> give it a dedicated bus or use an AHT20.

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

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"AHT20","host":"raspberrypi","temperature":22.4,"humidity":45.1}
```

`npm test` checks the raw-to-value conversion (20-bit humidity/temperature
split and busy-status handling) against golden values from the Python
node's formulas.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-aht20-node.service`.

## Deviations from the Python Node

- BLE transport falls back to Wi-Fi (see above); everything else —
  protocol bytes, timing, payloads, emulation — matches the Python node.
