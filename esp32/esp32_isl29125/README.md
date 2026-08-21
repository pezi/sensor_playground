# ESP32 SparkFun ISL29125 RGB Light Sensor Node for Sensor Playground

This Arduino project reads a **SparkFun RGB Light Sensor breakout** with the
Intersil/Renesas **ISL29125** (I2C address `0x44`) on an ESP32 and serves the
measured color and an approximate illuminance to the Sensor Playground app.
https://www.sparkfun.com/sparkfun-rgb-light-sensor-isl29125.html

The chip measures the intensity of red, green and blue light while rejecting
infrared from light sources.

> **Pollable, not push.** Unlike the digital contact sensors (button, PIR, …)
> the sensor produces continuous values, so it uses the same HTTPS REST + UDP
> discovery transport as the environment sensors. The app shows an
> "Illuminance" card/chart and a live color swatch.

## Readings

| JSON key (REST) | Meaning |
|-----------------|---------|
| `red`/`green`/`blue` | measured color, normalized against the brightest channel (`0`-`255`); absent in complete darkness |
| `lux` | approximate illuminance derived from the green channel, whose spectral response resembles the human eye (10K lux range, 16-bit) |

Unlike the TCS34725 the ISL29125 has **no clear channel**, so no color
temperature is reported.

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate and the `X-Api-Key` header. |
| `TRANSPORT_BLE`  | BLE GATT service. The app writes the API key to the auth characteristic, then reads/subscribes to the data characteristic. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **SparkFun RGB Light Sensor - ISL29125**

### Wiring (I2C)

| ESP32 Pin | Breakout Pin |
|-----------|--------------|
| 3.3V | 3.3V |
| GND | GND |
| GPIO 21 | SDA |
| GPIO 22 | SCL |

The breakout is a **3.3V** board without level shifting — do not feed it 5V.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `SparkFun ISL29125 Breakout` by SparkFun Electronics

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
{"sensor":"ISL29125","host":"ESP32","lux":428,"red":182,"green":255,"blue":97}
```
