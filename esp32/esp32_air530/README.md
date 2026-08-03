# ESP32 Air530 GPS Sensor Node for Sensor Tester

This Arduino project drives a **Grove GPS (Air530) module** on an ESP32 and
serves latitude, longitude, altitude and the satellites-in-use count to the
Sensor Tester app.

> **Serial, not I2C.** Like the CozIR, the Air530 is a UART device: it
> continuously streams standard NMEA-0183 sentences at 9600 baud. The sketch
> reads the stream on `Serial2` — no sensor library is needed.

## Sensor Protocol

The parser follows the dart_periphery `NmeaParser`
([serial_air530.dart](https://github.com/pezi/dart_periphery/blob/main/example/serial_air530.dart)):
each complete `GGA` sentence with a valid XOR checksum and a fix quality > 0
updates the cached position:

```
$GNGGA,062151.000,5005.12390,N,01811.91859,E,1,05,6.0,202.8,M,41.8,M,,*49
        time      lat        N  lon         E  fix sats hdop alt
```

Coordinates are converted from NMEA `ddmm.mmmm` to decimal degrees (WGS-84).
Readings older than 5 s are not served — until the module has a position fix
(cold start can take ~30 s with sky view) the JSON contains only the node
identity, and the BLE notify is skipped.

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + HTTPS REST (9132) with a self-signed certificate and the `X-Api-Key` header. |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Tester service UUID, writes the API key to the auth characteristic, then reads/subscribes to the data characteristic. |

The JSON payload is identical on both transports. The BLE GATT UUIDs are the
shared Sensor Tester contract (`d1a51b00-000{1,2,3}-…`, see the sketch) and
must match the app's `BleUuids`.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove GPS (Air530)** (Seeed Studio, Goke Air530 chip)

### Wiring (UART)

| ESP32 Pin | Module Pin |
|-----------|------------|
| 3.3V      | VCC        |
| GND       | GND        |
| GPIO 16 (RX2) | Tx     |
| GPIO 17 (TX2) | Rx     |

For a position fix the antenna needs sky view — expect no fix deep indoors.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon

   No sensor library is needed — the NMEA stream is parsed directly.

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
   that clients (the Sensor Tester app) must present.

### SSL Certificate (TRANSPORT_WIFI)

Run `../generate_cert.sh` to generate a self-signed certificate and insert
`SERVER_CERT` / `SERVER_KEY` into `secrets.h` automatically.

**Note:** `secrets.h` is excluded from Git to protect your credentials.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://<esp32-ip>:9132/
```

Returns e.g.:

```json
{"sensor":"AIR530","host":"ESP32","latitude":50.085398,"longitude":18.198643,"altitude":202.8,"satellites":5}
```
