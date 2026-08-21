# ESP32 DHT22 Sensor Node for Sensor Playground

This Arduino project implements the Sensor Playground
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with a DHT22
sensor (temperature, humidity) — the white
[Grove Temperature & Humidity Sensor Pro](https://wiki.seeedstudio.com/Grove-Temperature_and_Humidity_Sensor_Pro/)
module.

> The DHT22 (±0.5 °C / ±2–5 %RH, 0.1 resolution, −40…80 °C, one sample
> every two seconds) speaks the same proprietary single-wire protocol as
> the DHT11 on one digital pin, with better resolution and a wider range.
> The blue DHT11 module has its own sketch in [`../esp32_dht11/`](../esp32_dht11/).
>
> Single-wire reads occasionally fail even on a healthy sensor; the
> driver caches the last reading for two seconds and a failed read simply
> omits the value keys, which the app treats as "no fresh data" rather
> than an error.

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed cert. The app discovers and **polls** it. |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Playground service UUID, writes the API key to the auth characteristic, then **subscribes** to the data characteristic. No TLS certificate needed. |

The JSON payload is identical on both transports. The BLE GATT UUIDs are the
shared Sensor Playground contract (`d1a51b00-000{1,2,3}-…`, see the sketch) and
must match the app's `BleUuids`.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove Temperature & Humidity Sensor Pro** (DHT22 / AM2302, white)

### Wiring (single-wire digital)

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 3.3V or 5V | VCC (red) |
| GND | GND (black) |
| `DHT_PIN` (default GPIO 4) | SIG (yellow) |

> The Grove module already carries the required pull-up resistor on the
> signal line; add a 10 kΩ pull-up to VCC when wiring a bare DHT22.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `DHT sensor library` by Adafruit
   - `Adafruit Unified Sensor` (dependency of the DHT library)

### Quick install (arduino-cli)

Instead of the manual IDE setup above, you can build and flash with
[arduino-cli](https://arduino.github.io/arduino-cli/) using the bundled
script:

```bash
./install.sh <serial-port> [WIFI|BLE]

# Examples
./install.sh /dev/cu.usbserial-0001        # BLE (default)
./install.sh /dev/cu.usbserial-0001 WIFI   # Wi-Fi
```

The script installs the ESP32 core and all required libraries, creates
`secrets.h` from `secrets.h.example` on the first run (edit it, then
re-run), compiles the sketch with
`-DACTIVE_TRANSPORT=TRANSPORT_<WIFI|BLE>`, and uploads it to the given
serial port. Watch the serial log afterwards with:

```bash
arduino-cli monitor -p <serial-port> --config baudrate=115200
```

## Configuration

### Secrets

1. Copy `secrets.h.example` to `secrets.h`.
2. Edit `secrets.h` and enter your WiFi SSID, Password, and the API Key
   that clients (the Sensor Playground app) must present.

**Note:** `secrets.h` is excluded from Git to protect your credentials.

## How It Works

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts
  with a JSON packet containing the sensor type, IP, port, and current
  readings.
- **HTTPS REST API (port 9132):** Serves `GET /` with full sensor data
  over TLS (mbedTLS). Requires the `X-Api-Key` header to match the
  configured key.

### SSL/TLS

The certificate and private key are stored in `secrets.h` as
`SERVER_CERT` and `SERVER_KEY`. Run the provided script to generate
them automatically:

```bash
cp secrets.h.example secrets.h   # if not done already
../generate_cert.sh
```

This generates a self-signed RSA-2048 certificate (valid 10 years) and
writes it into `secrets.h` in the correct C string format.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://<esp32-ip>:9132/
```

Expected response:

```json
{"sensor":"DHT22","host":"ESP32","temperature":22.4,"humidity":46.3}
```

## See also

`../../python/dht22/` — the same node for Raspberry Pi & co.
