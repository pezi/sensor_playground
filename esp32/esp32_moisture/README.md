# ESP32 Grove Capacitive Moisture Sensor Node for Sensor Playground

This Arduino project reads a **Grove Capacitive Moisture Sensor
(Corrosion-Resistant)** on an ESP32 and serves the soil moisture as a
percentage to the Sensor Playground app.
https://wiki.seeedstudio.com/Grove-Capacitive_Moisture_Sensor-Corrosion-Resistant/

The probe is capacitive: no exposed metal electrodes, so unlike a resistive
probe it can stay in damp soil permanently without corroding. Its analog
output voltage **falls** as the soil gets wetter.

> **Pollable, not push.** Unlike the digital contact sensors (button, PIR, …)
> the probe produces a continuous value, so it uses the same HTTPS REST
> + UDP discovery transport as the environment sensors. The app shows it as a
> normal live-data metric (a "Soil Moisture" card and chart).

## Reading & calibration

The node reports `moisture` — a `0`-`100` % value mapped linearly between two
calibration points in the sketch:

| Constant | Meaning | Default |
|----------|---------|---------|
| `ADC_DRY` | averaged raw ADC reading in dry air | `2600` |
| `ADC_WET` | averaged raw ADC reading submerged in water | `1100` |

Probes and supply voltages vary, so **calibrate yours**: the REST payload also
carries the averaged raw reading (`adc`, with its full scale `adcMax`, 4095 on
the ESP32's 12-bit ADC). Hold the probe in dry air, read `adc`, put the number
into `ADC_DRY`; submerge it up to the marked line in a glass of water and do
the same for `ADC_WET`. Values outside the calibrated span clamp to 0/100 %.

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate and the `X-Api-Key` header. |
| `TRANSPORT_BLE`  | BLE GATT service. The app writes the API key to the auth characteristic, then reads/subscribes to the data characteristic. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove Capacitive Moisture Sensor (Corrosion-Resistant)**

### Wiring (analog)

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 3.3V | VCC |
| GND | GND |
| GPIO 34 (ADC1, input-only) | SIG (yellow) |

Do not insert the probe deeper than the marked line — the electronics at the
top are not waterproof.

GPIO 34 is on ADC1, which keeps working while Wi-Fi is active (ADC2 does not).
Change `MOISTURE_PIN` if you wire it elsewhere — stay on an ADC1 pin.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon

   No sensor library is needed — the probe is read with `analogRead`.

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
{"sensor":"MOISTURE","host":"ESP32","moisture":63,"adc":1820,"adcMax":4095}
```
