# ESP32 SHT11 Sensor Node for Sensor Playground

This Arduino project implements the Sensor Playground
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with a Sensirion
SHT11 sensor (temperature, humidity) — the classic SHT1x family.

> The SHT10, SHT11 and SHT15 differ only in calibration accuracy (±0.5 /
> ±0.4 / ±0.3 °C typical) and speak the same proprietary **two-wire
> protocol** (SCK + bidirectional DATA). It resembles I2C but is *not*
> I2C — the sensor cannot share an I2C bus with other devices. Set
> `SENSOR_NAME` in the sketch to the chip on your board so the app shows
> the right name; nothing else changes.
>
> The protocol is implemented directly in the sketch (no sensor library):
> the bus is fully master-clocked with no minimum speed, so bit-banging is
> reliable. The sensor must not be measured more than ~10% of the time or
> it heats itself; the sketch reads at most every two seconds and caches
> the values, and a failed read serves the previous reading for up to 30
> seconds before the value keys are omitted (which the app treats as "no
> fresh data" rather than an error).

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed cert. The app discovers and **polls** it. |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Playground service UUID, writes the API key to the auth characteristic, then **subscribes** to the data characteristic. No TLS certificate needed. |

The JSON payload is identical on both transports. The BLE GATT UUIDs are the
shared Sensor Playground contract (`d1a51b00-000{1,2,3}-…`, see the sketch) and
must match the app's `BleUuids`.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **SHT10 / SHT11 / SHT15** module or bare sensor

### Wiring (two-wire, NOT I2C)

| ESP32 Pin | Sensor Pin |
|-----------|------------|
| 3.3V | VDD |
| GND | GND |
| `SHT_DATA_PIN` (default GPIO 4) | DATA |
| `SHT_SCK_PIN` (default GPIO 5) | SCK |

> The DATA line needs a pull-up resistor (~10 kΩ to VDD); most breakout
> boards carry one, and the sketch additionally enables the ESP32's
> internal pull-up. The datasheet recommends 3.3V supply — the
> temperature conversion constant in the sketch (`SHT1X_D1 = -39.66`)
> assumes it.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - (no sensor library — the SHT1x protocol lives in the sketch)

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
curl -k -H "X-Api-Key: your-sensor-api-key" https://<esp32-ip>:9132/
```

Expected response:

```json
{"sensor":"SHT11","host":"ESP32","temperature":22.4,"humidity":46.1}
```

## See also

`../../python/sht11/` — the same node for Raspberry Pi & co.
