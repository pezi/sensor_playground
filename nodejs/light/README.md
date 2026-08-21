# Grove Light Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a Grove Light Sensor — an analog photo-resistor reporting a raw
brightness value (higher = brighter) under the JSON key `light`. The
Sensor Playground app discovers this node via UDP broadcast (port 9133)
and polls it for data over HTTPS (port 9132, `X-Api-Key` header).

It is the Node.js counterpart of [`../../python/light/`](../../python/light/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Needs a Grove Base Hat

The Raspberry Pi has no analog input, so the sensor is read through the
Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04): plug the sensor
into one of the hat's analog ports (A0-A7) and set `pin` to that channel.
Enable I2C: `sudo raspi-config` → Interface Options → I2C.

> The Arduino-based hats the Python node also supports (`"nano"`,
> `"grovePlus"`) are **not** implemented in this port — use the Python
> node for those.

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, pin
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
{"sensor":"LIGHT","host":"raspberrypi","light":742}
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-light-node.service`.
