# ESP32 Grove Dust Sensor (PPD42NS) Node for Sensor Tester

This Arduino project reads a **Grove Dust Sensor** (Shinyei PPD42NS) on an
ESP32 and serves a particle concentration to the Sensor Tester app.
https://wiki.seeedstudio.com/Grove-Dust_Sensor/

> **Pollable, not push.** Unlike the digital contact sensors (button, PIR, …)
> the dust value is continuous, so it uses the same HTTPS REST + UDP discovery
> transport as the environment sensors. The app shows it as a classified
> air-quality card (Dylos bands) plus a live chart.

## Reading

The PPD42NS pulls its output pin LOW while particles scatter light inside its
chamber (pulses of roughly 10–90 ms). The sketch times both edges with a
pin-change interrupt and accumulates the **low-pulse occupancy (LPO)** over
30-second windows:

    ratio         = low_time / window_time * 100          (percent)
    concentration = 1.1·r³ − 3.8·r² + 520·r + 0.62        (pcs/0.01cf)

(the Nafis curve, https://www.howmuchsnow.com/arduino/airquality/grovedust/).
The value is served under the short/long JSON key `dust`.

- **The first reading appears after the first full 30-second window.** Until
  then the payloads carry no `dust` key (and the BLE build sends no notifies)
  — that is warm-up, not a fault.
- Shinyei recommends letting the sensor stabilize for ~3 minutes after
  power-on before trusting the values.

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate and the `X-Api-Key` header. |
| `TRANSPORT_BLE`  | BLE GATT service. The app writes the API key to the auth characteristic, then reads/subscribes to the data characteristic. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove Dust Sensor (Shinyei PPD42NS)**
- Two resistors for a voltage divider (e.g. 10 kΩ + 20 kΩ)

### Wiring (digital, 5 V sensor!)

| PPD42NS pin | Connect to |
|-------------|------------|
| 1 (GND, black) | GND |
| 3 (5V, red) | **VIN / 5 V** (the heater draws ~90 mA — do not use 3V3) |
| 4 (P1 output, yellow) | voltage divider → GPIO 34 |

The output swings up to ~4.5 V, which is **not 3.3 V-safe**: pass it through a
voltage divider (e.g. 10 kΩ from the sensor output to the GPIO, 20 kΩ from
the GPIO to GND ⇒ 4.5 V → 3.0 V). GPIO 34 is input-only and needs no internal
pull — the divider drives it. Change `DUST_PIN` if you wire it elsewhere.

Mount the sensor vertically — its chamber relies on the heater's convection
air flow.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon

   No sensor library is needed — the pulses are timed with a pin-change
   interrupt.

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
{"sensor":"PPD42NS","host":"ESP32","dust":412.5}
```

During the first 30 seconds after boot the reply carries no `dust` key — the
first LPO window has not completed yet.
