# ESP32 Grove Light Sensor Node for Sensor Tester

This Arduino project reads a **Grove Light Sensor** (an analog photo-resistor)
on an ESP32 and serves a raw brightness value to the Sensor Tester app.
https://wiki.seeedstudio.com/Grove-Light_Sensor/

> **Pollable, not push.** Unlike the digital contact sensors (button, PIR, …)
> the light sensor produces a continuous value, so it uses the same HTTPS REST
> + UDP discovery transport as the environment sensors. The app shows it as a
> normal live-data metric (a "Brightness" card and chart).

## Reading

The value is the raw 12-bit ADC reading (`0`-`4095`) under the short/long JSON
key `light` — higher means brighter. It is **not** calibrated to lux; it is a
relative brightness, consistent with the dart_periphery example which prints
the raw `analogRead` / `readADCraw` value.

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate and the `X-Api-Key` header. |
| `TRANSPORT_BLE`  | BLE GATT service. The app writes the API key to the auth characteristic, then reads/subscribes to the data characteristic. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove Light Sensor**

### Wiring (analog)

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 3.3V | VCC |
| GND | GND |
| GPIO 34 (ADC1, input-only) | SIG (yellow) |

GPIO 34 is on ADC1, which keeps working while Wi-Fi is active (ADC2 does not).
Change `LIGHT_PIN` if you wire it elsewhere — stay on an ADC1 pin.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon

   No sensor library is needed — the photo-resistor is read with `analogRead`.

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
{"sensor":"LIGHT","host":"ESP32","light":2731}
```
