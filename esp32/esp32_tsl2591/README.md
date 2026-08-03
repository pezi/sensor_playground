# ESP32 TSL2591 Node for Sensor Tester

Reads a [TSL2591 light sensor](https://learn.adafruit.com/adafruit-tsl2591) and
reports **visible light, infrared and illuminance** to the Sensor Tester app.

It covers roughly **188 µlx to 88 klx** — moonlight to direct sun — which is
about 600 000 000 : 1. No single gain setting spans that, so the sketch shifts
gain to follow the light (see below).

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet plus the current readings under their short keys
  (`vis`, `ir`, `lux`), so the scan list previews them.
- **HTTPS REST (port 9132):** `GET /` with the shared `X-Api-Key` header
  returns the full-key payload:

  ```json
  {"sensor":"TSL2591","host":"ESP32","visible":4231,"ir":1187,"lux":312.4}
  ```

| Key | Unit | Meaning |
|-----|------|---------|
| `visible` | counts | Broadband channel minus the infrared channel. |
| `ir` | counts | Infrared channel (channel 1). |
| `lux` | lx | Calibrated illuminance, computed from both channels. |

The app needs no per-sensor code: all three quantities are already in its
metric registry, `visible` and `ir` shared with the SI1145 and `lux` with the
TCS34725.

## Visible light, infrared and lux

The chip has two photodiodes. Channel 0 is **broadband** — it sees visible
light *and* infrared. Channel 1 is **infrared only**. Neither is "visible
light" on its own; the difference of the two is:

```
visible = broadband - ir
```

The sketch floors that at zero, because in near-darkness noise can put the IR
channel marginally above the broadband one, and a negative amount of visible
light would be nonsense on a chart.

`lux` is a different thing from either count. The driver applies the datasheet's
coefficients to both channels and divides out the current gain and integration
time, which is why those two settings must be given to the driver and not only
to the chip. Both counts are reported alongside it so that a saturated or
IR-heavy scene is visible rather than hidden behind a single number.

**A saturated reading has no lux.** When a channel pins at 0xFFFF the driver
returns a negative lux; the sketch then omits the `lux` key and sends only the
counts, rather than publishing a number it knows to be wrong. The app's metric
registry renders whatever keys are present, so the card simply disappears for
that reading.

## Gain and integration time

| Constant | Default | Meaning |
|----------|---------|---------|
| `DEFAULT_GAIN` | `TSL2591_GAIN_MED` (25x) | Sensitivity. `GAIN_LOW` 1x, `GAIN_MED` 25x, `GAIN_HIGH` 428x, `GAIN_MAX` 9876x. |
| `DEFAULT_INTEGRATION` | `TSL2591_INTEGRATIONTIME_300MS` | 100–600 ms. Longer collects more light and resolves finer, but each reading takes that long. |
| `AUTO_GAIN` | `true` | Follow the light level by shifting gain — see below. |

25x with 300 ms suits normal indoor light. Outdoors in daylight use `GAIN_LOW`
with 100 ms; for moonlight use `GAIN_MAX`.

With `AUTO_GAIN` the sketch moves **one step at a time**: up when the broadband
channel falls to `TOO_DARK_COUNTS` (100), down when it saturates. One step,
because changing the gain invalidates the integration already in flight — the
*next* reading is the one that benefits — and because jumping straight to the
extreme makes the value oscillate whenever the light sits near a threshold.
Set `AUTO_GAIN` to `false` when you want readings taken at one fixed
sensitivity, e.g. to compare counts across a session.

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate. |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, then reads or subscribes to the data characteristic for the same JSON payload. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- TSL2591 breakout (Adafruit 1980 or compatible)

### Wiring

| ESP32 Pin | Sensor Pin |
|-----------|-----------|
| 3.3V | VIN |
| GND | GND |
| GPIO 22 (SCL) | SCL |
| GPIO 21 (SDA) | SDA |

The I2C address is **fixed at 0x29** — there is no address pin. That is the
same address the **TCS34725** uses, so those two sensors cannot share a bus.

## Configuration

Copy `secrets.h.example` to `secrets.h` and fill in WiFi, API key and — for the
Wi-Fi transport — the TLS certificate:

```bash
../generate_cert.sh          # writes SERVER_CERT / SERVER_KEY into secrets.h
```

`secrets.h` is excluded from Git.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `Adafruit TSL2591 Library`
   - `Adafruit Unified Sensor`

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

Sanity checks: a lit living room reads a few hundred lux, an office around
500 lx, and covering the sensor with a hand should drop it to single digits.
Point a TV remote at it and press a button — `ir` jumps while `visible`
barely moves.

## See also

`../../python/tsl2591/` — the same node for Raspberry Pi & co.
