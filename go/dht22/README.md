# DHT22 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
Raspberry Pi and similar Linux boards with a DHT22/AM2302 sensor — including
the white [Grove Temperature & Humidity Sensor Pro](https://wiki.seeedstudio.com/Grove-Temperature_and_Humidity_Sensor_Pro/).
It is the Go counterpart of [`../../python/dht22/`](../../python/dht22/).

The node exposes HTTPS REST on port 9132 and UDP discovery on port 9133.
BLE is not supported by the Go transport; a BLE configuration falls back to
Wi-Fi with a warning. Use the Python or Rust port when BLE is required.

The DHT22's 26–70 µs single-wire pulses are decoded from kernel-timestamped
GPIO edge events. Reads are limited to one attempt every two seconds and the
last successful reading is served for up to 30 seconds to tolerate normal
single-wire read failures.

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
go build -o sensor_node_dht22 .
cp config.example.json config.json
```

Set `api_key`, `gpio_pin`, and `gpio_chip` (`0` on a typical Raspberry Pi).
Generate Wi-Fi certificates with:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

Run with `./sensor_node_dht22`. Set `"emulation": true` to generate
one-decimal DHT22-style readings without hardware.

## Test

```bash
go test
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

Expected payload:

```json
{"sensor":"DHT22","host":"raspberrypi","temperature":22.4,"humidity":46.3}
```

The blue DHT11 module has its own Go node in [`../dht11/`](../dht11/).
