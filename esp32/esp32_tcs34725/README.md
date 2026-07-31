# ESP32 TCS34725 Color Sensor Node for Sensor Tester

This Arduino project implements the Sensor Tester
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with a
[Grove I2C Color Sensor V2](https://wiki.seeedstudio.com/Grove-I2C_Color_Sensor/)
(TCS34725: RGB color, color temperature, illuminance) via I2C.

> The node reports the measured **color** as RGB channels normalized to
> 0–255 (shown as a swatch in the app), plus **color temperature** (Kelvin)
> and **illuminance** (lux) computed by the Adafruit driver.

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed cert. The app discovers and **polls** it. |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Tester service UUID, writes the API key to the auth characteristic, then **subscribes** to the data characteristic. No TLS certificate needed. |

The JSON payload is identical on both transports. The BLE GATT UUIDs are the
shared Sensor Tester contract (`d1a51b00-000{1,2,3}-…`, see the sketch) and
must match the app's `BleUuids`.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove I2C Color Sensor V2** (TCS34725)

### Wiring (I2C)

| ESP32 Pin | Sensor Pin |
|-----------|------------|
| 3.3V      | VCC        |
| GND       | GND        |
| GPIO 21   | SDA        |
| GPIO 22   | SCL        |

> The TCS34725 uses fixed I2C address `0x29`.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `Adafruit TCS34725`

## Configuration

### Secrets

1. Copy `secrets.h.example` to `secrets.h`.
2. Edit `secrets.h` and enter your WiFi SSID, Password, and the API Key
   that clients (the Sensor Tester app) must present.

**Note:** `secrets.h` is excluded from Git to protect your credentials.

## How It Works

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts
  with a JSON packet containing the sensor type, IP, port, and the current
  color temperature / illuminance.
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
{"sensor":"TCS34725","host":"ESP32","colorTemperature":4480,"lux":312,"red":142,"green":188,"blue":201}
```
