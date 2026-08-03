# ESP32 BMP085 Node for Sensor Tester

Reads a [BMP085 barometer](https://wiki.seeedstudio.com/Grove-Barometer_Sensor/)
— the sensor behind the Grove Barometer Sensor, and the ancestor of the BME280
— and reports **temperature, barometric pressure and altitude** to the Sensor
Tester app.

The pin-compatible **BMP180** uses the same registers and works with this
sketch unchanged.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet plus the current readings under their short keys
  (`temp`, `press`, `alt`), so the scan list previews them.
- **HTTPS REST (port 9132):** `GET /` with the shared `X-Api-Key` header
  returns the full-key payload:

  ```json
  {"sensor":"BMP085","host":"ESP32",
   "temperature":21.3,"pressure":1013.25,"altitude":-0.1}
  ```

| Key | Unit | Meaning |
|-----|------|---------|
| `temperature` | °C | Die temperature. |
| `pressure` | hPa | Compensated barometric pressure. |
| `altitude` | m | Derived from the pressure — see below. |

The app needs no per-sensor code for any of these: all three are already in its
metric registry, shared with the BME280 (temperature, pressure) and the GPS
node (altitude).

## About the altitude

It is computed from the pressure against the standard sea-level value of
**1013.25 hPa**, using the international barometric formula the datasheet
quotes. That makes it an absolute altitude only when the weather happens to
match the standard atmosphere — on a low-pressure day a stationary sensor
reads a few dozen metres too high. As a *relative* measurement (lift the
sensor, watch it climb) it is precise to well under a metre.

## Why there is no sensor library

The BMP085 returns **uncompensated** readings: two raw ADC values that only
become a temperature and a pressure after Bosch's fixed-point compensation,
using eleven calibration constants read from the chip's EEPROM. That
arithmetic is in `bmp085Compensate()` here and mirrored in
`../../python/bmp085/sensor_node.py`, so both nodes produce identical numbers
from identical raw readings — which is easier to keep true with the two copies
side by side than with a library on one side and a transcription on the other.

It is verified against the worked example published in the datasheet
(section 3.5): calibration `AC1=408 … MD=2868` with `UT=27898`, `UP=23843`
must yield exactly **15.0 °C** and **69964 Pa**.

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate. |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, then reads or subscribes to the data characteristic for the same JSON payload. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- BMP085 or BMP180 breakout / Grove Barometer Sensor

### Wiring

| ESP32 Pin | Sensor Pin |
|-----------|-----------|
| 3.3V | VCC |
| GND | GND |
| GPIO 22 (SCL) | SCL |
| GPIO 21 (SDA) | SDA |

The I2C address is **fixed at 0x77** — the BMP085 has no address pin, so only
one can sit on a bus.

## Configuration

| Constant | Default | Meaning |
|----------|---------|---------|
| `OVERSAMPLING` | `3` | 0–3. Higher means less noise and a longer conversion (5 ms at 0, 26 ms at 3). 3 is the datasheet's "ultra high resolution" mode. |

Then copy `secrets.h.example` to `secrets.h` and fill in WiFi, API key and —
for the Wi-Fi transport — the TLS certificate:

```bash
../generate_cert.sh          # writes SERVER_CERT / SERVER_KEY into secrets.h
```

`secrets.h` is excluded from Git.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon

   No sensor library — see above.

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

## Testing

```bash
curl -k -H 'X-Api-Key: your-sensor-api-key' https://<esp32-ip>:9132/
```

Sanity checks: the pressure should read within a few hPa of your local
airport's QNH, and the altitude should climb by roughly 8 m for every 1 hPa
the pressure drops.

## See also

`../../python/bmp085/` — the same node for Raspberry Pi & co.
