# ESP32 Chirp I2C Soil Moisture Sensor Node for Sensor Playground

This Arduino project reads a **Catnip Electronics I2C Soil Moisture Sensor**
(the "Chirp" sensor, default I2C address `0x20`) on an ESP32 and serves the
soil moisture as a percentage, the soil temperature and the ambient light
level to the Sensor Playground app.

- Sensor library and protocol: https://github.com/Apollon77/I2CSoilMoistureSensor
- Product: https://www.robotshop.com/products/i2c-soil-moisture-sensor

The sensor is capacitive: no exposed metal electrodes, so it can stay in
soil permanently without corroding. Unlike the analog Grove probe it talks
plain I2C — no ADC involved.

> **Pollable, not push.** The sensor produces continuous values, so it uses
> the same HTTPS REST + UDP discovery transport as the environment sensors.
> The app shows "Soil Moisture", "Temperature" and "Brightness" cards/charts.

## Readings & calibration

| JSON key (REST) | Meaning |
|-----------------|---------|
| `moisture` | soil moisture, `0`-`100` %, mapped between `CAP_DRY`/`CAP_WET` |
| `temperature` | soil/sensor temperature in °C (chip reports tenths) |
| `light` | ambient brightness in raw counts, **higher = brighter** (the chip counts a phototransistor discharge upward in darkness; the sketch inverts it, `65535 - raw`) |
| `cap` | the raw capacitance, for calibrating the two points below |

The raw capacitance rises with moisture, so **wet > dry**:

| Constant | Meaning | Default |
|----------|---------|---------|
| `CAP_DRY` | raw capacitance in dry air | `290` |
| `CAP_WET` | raw capacitance submerged in water | `520` |

Individual sensors vary, so **calibrate yours**: read `cap` from the REST
payload with the sensor in dry air and again submerged up to the marked line
in a glass of water, and put the two numbers into the sketch. Values outside
the calibrated span clamp to 0/100 %.

A light measurement takes up to three seconds on the chip, so it runs as a
non-blocking state machine in the loop; the payload omits the `light` key
until the first measurement has completed (a few seconds after boot).

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate and the `X-Api-Key` header. |
| `TRANSPORT_BLE`  | BLE GATT service. The app writes the API key to the auth characteristic, then reads/subscribes to the data characteristic. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Catnip Electronics I2C Soil Moisture Sensor**

### Wiring (I2C)

| ESP32 Pin | Sensor Pin |
|-----------|------------|
| 3.3V | VCC |
| GND | GND |
| GPIO 21 | SDA |
| GPIO 22 | SCK/SCL |

Only the coated probe part goes into the soil — keep the components at the
top dry.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `I2CSoilMoistureSensor` by Ingo Fischer

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

1. Copy `secrets.h.example` to `secrets.h` and fill in WiFi + API key.
2. For `TRANSPORT_WIFI`, run `../generate_cert.sh` to insert `SERVER_CERT` /
   `SERVER_KEY` into `secrets.h`.

`secrets.h` is excluded from Git.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://<esp32-ip>:9132/
```

Returns e.g.:

```json
{"sensor":"CHIRP","host":"ESP32","moisture":47,"temperature":21.3,"light":52000,"cap":400}
```
