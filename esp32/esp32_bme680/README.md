# ESP32 BME680 Sensor Node for Sensor Playground

This Arduino project implements the Sensor Playground
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with a BME680 I2C
sensor (temperature, humidity, pressure, IAQ).

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed cert. The app discovers and **polls** it. |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Playground service UUID, writes the API key to the auth characteristic, then **subscribes** to the data characteristic. No TLS certificate needed. |

The JSON payload is identical on both transports. The BLE GATT UUIDs are the
shared Sensor Playground contract (`d1a51b00-000{1,2,3}-…`, see the sketch) and must
match the app's `BleUuids`.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **BME680** breakout (temperature, humidity, pressure, IAQ)

The sketch detects BME680 boards at either supported I2C address (`0x76` or
`0x77`).

### Wiring (I2C)

| ESP32 Pin | Sensor Pin |
|-----------|------------|
| 3.3V      | VCC        |
| GND       | GND        |
| GPIO 21   | SDA        |
| GPIO 22   | SCL        |

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `Adafruit BME680 Library`

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

The script installs pinned versions of the ESP32 core and all required
libraries, creates
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
# Hardware-independent gas-valid/IAQ regression test
c++ -std=c++17 -Wall -Wextra -Werror tests/test_bme680_logic.cpp -o /tmp/bme680_logic_test
/tmp/bme680_logic_test

# Live node smoke test
curl -k -H "X-Api-Key: your-sensor-api-key" https://<esp32-ip>:9132/
```

The sketch reads the BME680 field status directly because Adafruit BME680
2.0.6 does not expose it and accepts either gas-valid flag. IAQ advances only
when both gas-valid and heater-stable are set. BLE callbacks serve a protected
cached payload; only the main loop performs BLE-mode sensor measurements.

## IAQ Calculation

The BME680 air-quality score is computed using a rolling-baseline algorithm
(ported from [dart_periphery](https://pub.dev/packages/dart_periphery)).
It maintains a window of 50 gas resistance readings to establish a
baseline, then scores gas (75%) and humidity (25%) relative to their
baselines. Only readings marked gas-valid and heater-stable are accepted, so
warm-up or failed heater readings do not contaminate the baseline. The score
uses a `0..100` scale where higher is better (it is not Bosch's BSEC IAQ
`0..500` index) and stabilizes after approximately 50 valid readings.
