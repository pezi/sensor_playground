# ESP32 BME280 Sensor Node for Sensor Tester

This Arduino project implements the Sensor Tester
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with a BME280 I2C
sensor (temperature, humidity, pressure).

> The BME280 measures temperature, humidity and pressure only. Unlike the
> BME680 it has no gas sensor, so it reports no air-quality (IAQ) value.

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed cert. The app discovers and **polls** it. |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for `SERVICE_UUID`, writes the API key to the auth characteristic, then **subscribes** to the data characteristic. |

The JSON payload is identical on both transports, so the app parses it the same
way. The BLE GATT UUIDs must match the app's `BleUuids`:

| Characteristic | UUID | Use |
|----------------|------|-----|
| Service        | `d1a51b00-0001-…` | advertised; scanned for |
| Data (read/notify) | `d1a51b00-0002-…` | JSON reading |
| Auth (write)   | `d1a51b00-0003-…` | client writes the API key |

For `TRANSPORT_BLE` you do **not** need a TLS certificate (the `SERVER_CERT` /
`SERVER_KEY` fields in `secrets.h` are unused).

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **BME280** module (temperature, humidity, pressure)

### Wiring (I2C)

| ESP32 Pin | Sensor Pin |
|-----------|------------|
| 3.3V      | VCC        |
| GND       | GND        |
| GPIO 21   | SDA        |
| GPIO 22   | SCL        |

> Most BME280 breakouts use I2C address `0x76`; some use `0x77`. Adjust
> `BME280_ADDR` in the sketch if the board is not found at boot.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `Adafruit BME280 Library` (pulls in `Adafruit Unified Sensor`)

## Configuration

### Secrets

1. Copy `secrets.h.example` to `secrets.h`.
2. Edit `secrets.h` and enter your WiFi SSID, Password, and the API Key
   that clients (the Sensor Tester app) must present.

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
{"sensor":"BME280","host":"ESP32","temperature":22.4,"humidity":45.1,"pressure":1013.2}
```
