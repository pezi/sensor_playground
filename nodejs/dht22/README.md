# DHT22 Sensor Node for Sensor Playground (Node.js)

This Node.js program exposes a DHT22/AM2302 temperature and humidity node over
HTTPS REST (port 9132) with UDP discovery (port 9133). It is the Node.js
counterpart of [`../../python/dht22/`](../../python/dht22/) and defaults to the
sensor name `DHT22`.

> **Emulation only.** DHT22 bits are distinguished by 26–28 µs versus 70 µs
> pulses. The available Node.js GPIO paths do not expose the kernel edge
> timestamps required to decode them reliably. Use the Python, Go, or Rust
> port for real hardware. BLE is also unavailable in the Node.js transport;
> `"transport": "ble"` falls back to Wi-Fi with a warning.

## Setup

```bash
npm install
cp config.example.json config.json
```

Set `api_key` and change `"emulation"` to `true`. The `gpio_pin` field is
accepted for configuration parity but is unused by this emulation-only port.

Generate the HTTPS certificate and key:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

Run with `node sensor_node.js`. The node generates plausible indoor readings
at the DHT22's one-decimal resolution.

## Test

```bash
npm test
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

Expected payload:

```json
{"sensor":"DHT22","host":"raspberrypi","temperature":22.4,"humidity":46.3}
```

The blue DHT11 module has its own Node.js node in [`../dht11/`](../dht11/).
