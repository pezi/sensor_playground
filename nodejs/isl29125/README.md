# ISL29125 RGB Light Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a **SparkFun RGB Light
Sensor breakout** — an Intersil/Renesas **ISL29125** (I2C address `0x44`).
The Sensor Playground app discovers this node via UDP broadcast (port 9133)
and polls it for data over HTTPS (port 9132, `X-Api-Key` header).
https://www.sparkfun.com/sparkfun-rgb-light-sensor-isl29125.html

It is the Node.js counterpart of [`../../python/isl29125/`](../../python/isl29125/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python node for BLE).

The driver is a straight port of the register access in the Python node
(smbus2): device-ID check and reset, RGB sampling in the 10,000 lux range
with maximum IR compensation, and a 6-byte block read of the green/red/blue
counts from register `0x09` — readings match the Python node.

## Readings

| JSON key (REST) | Meaning |
|-----------------|---------|
| `red`/`green`/`blue` | measured colour, normalized against the brightest channel (`0`-`255`); absent in complete darkness |
| `lux` | approximate illuminance derived from the green channel, whose spectral response resembles the human eye (10K lux range, 16-bit) |

Unlike the TCS34725 the ISL29125 has **no clear channel**, so no colour
temperature is reported. The UDP discovery reply carries only `lux`.

## Wiring (I2C, address 0x44)

| Pi Pin | Breakout Pin |
|--------|--------------|
| 3.3V (Pin 1) | 3.3V |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

The breakout is a **3.3V** board without level shifting — do not feed it 5V.

Enable I2C: `sudo raspi-config` → Interface Options → I2C. Other boards:
set `i2c_bus` (Raspberry Pi `1`, NanoPi `0`, Banana Pi `2`).

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, i2c_bus (address is decimal: 68 = 0x44)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves a plausible indoor-light scene shifting between
warm and cool white. Works on any machine (macOS/Windows included).

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
{"sensor":"ISL29125","host":"raspberrypi","lux":428,"red":182,"green":255,"blue":97}
```

`npm test` checks the counts → payload derivation (normalization against
the brightest channel, the lux factor, and no colour keys in darkness).

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-isl29125-node.service`.
