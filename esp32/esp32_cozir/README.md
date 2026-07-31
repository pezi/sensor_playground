# ESP32 CozIR CO₂ Sensor Node for Sensor Tester

This Arduino project drives a **CozIR-A CO₂ sensor** on an ESP32 and serves
temperature, humidity and CO₂ readings to the Sensor Tester app.

> **Serial, not I2C.** Unlike the other environment sensors the CozIR talks a
> simple ASCII command protocol over a **9600-baud UART**. The sketch drives
> it on `Serial2` — no sensor library is needed.

## Sensor Protocol

Commands from the
[dart_periphery serial_cozir.dart example](https://github.com/pezi/dart_periphery/blob/main/example/serial_cozir.dart):

| Command | Effect |
|---------|--------|
| `M 4164` | Select humidity, temperature and CO₂ output fields |
| `K 2` | Polling mode |
| `Q` | One measurement: `H 00495 T 01234 Z 06399` → 49.5 %RH, 23.4 °C, 639.9 ppm |

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
- **CozIR-A CO₂ sensor** (Gas Sensing Solutions)

### Wiring (UART, 3.3 V)

| ESP32 Pin | Sensor Pin |
|-----------|------------|
| 3.3V      | VCC        |
| GND       | GND        |
| GPIO 16 (RX2) | Tx     |
| GPIO 17 (TX2) | Rx     |

The CozIR is a 3.3 V device — do **not** connect it to 5 V.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon

   No sensor library is needed — the CozIR is driven over `Serial2`.

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
{"sensor":"COZIR","host":"ESP32","temperature":23.4,"humidity":49.5,"co2":639.9}
```
