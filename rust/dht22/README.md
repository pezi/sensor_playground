# DHT22 Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface for a
DHT22/AM2302 sensor, including the white
[Grove Temperature & Humidity Sensor Pro](https://wiki.seeedstudio.com/Grove-Temperature_and_Humidity_Sensor_Pro/).
It is the Rust counterpart of [`../../python/dht22/`](../../python/dht22/).

Both transports are supported on Linux:

- Wi-Fi: HTTPS REST on port 9132 plus UDP discovery on port 9133.
- BLE: the shared Sensor Playground GATT service with API-key authentication.

The DHT22's 26–70 µs pulses are decoded from kernel-timestamped GPIO edge
events. The line is reconfigured in place between the low start signal and
falling-edge capture. Reads are attempted at most every two seconds and the
last successful sample remains valid for 30 seconds.

## Wiring

| Pi pin | Grove/bare sensor |
|---|---|
| 3.3V (pin 1) | VCC |
| GND (pin 6) | GND |
| GPIO 4 (pin 7) | SIG/DATA |

The Grove module includes a pull-up. Add a 10 kΩ pull-up to 3.3V for a bare
DHT22 when needed.

## Build and configure

```bash
cargo build --release
cp config.example.json config.json
```

Set `api_key`, `gpio_pin`, `gpio_chip`, and the desired transport. Wi-Fi mode
also needs `cert.pem` and `key.pem`:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

Run `../target/release/sensor_node_dht22`. Set `"emulation": true` to
generate one-decimal DHT22-style readings without hardware.

## Test

```bash
cargo test -p sensor-playground-dht22
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

Expected payload:

```json
{"sensor":"DHT22","host":"raspberrypi","temperature":22.4,"humidity":46.3}
```

BLE requires Linux/BlueZ and the setup documented in
[`../common/README.md`](../common/README.md). The blue DHT11 module has its own
Rust node in [`../dht11/`](../dht11/).
