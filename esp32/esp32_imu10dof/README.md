# ESP32 IMU 10DOF Sensor Node for Sensor Tester

This Arduino project implements the Sensor Tester
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with a
[Grove IMU 10DOF](https://wiki.seeedstudio.com/Grove-IMU_10DOF/) board
(MPU9250 + BMP280) via I2C.

> The MPU9250's accelerometer, gyroscope and magnetometer are fused into
> **roll**, **pitch** and compass **heading** angles plus the total
> acceleration magnitude (**g-force**, ~1 g at rest). The BMP280 adds
> **temperature** and barometric **pressure**. The magnetometer benefits
> from a calibration — see the hideakitai MPU9250 library docs.

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | UDP discovery (9133) + WebSocket push (9132, `ws://`). The app discovers it, then **streams** readings pushed every 250 ms. No TLS certificate needed. |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Tester service UUID, writes the API key to the auth characteristic, then **subscribes** to the data characteristic. No TLS certificate needed. |

The JSON payload is identical on both transports. The BLE GATT UUIDs are the
shared Sensor Tester contract (`d1a51b00-000{1,2,3}-…`, see the sketch) and
must match the app's `BleUuids`.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove IMU 10DOF v2.0** (MPU9250 at `0x68`, BMP280 at `0x77`)

### Wiring (I2C)

| ESP32 Pin | Sensor Pin |
|-----------|------------|
| 3.3V      | VCC        |
| GND       | GND        |
| GPIO 21   | SDA        |
| GPIO 22   | SCL        |

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `MPU9250` by hideakitai
   - `Adafruit BMP280 Library` (drives the BMP280)
   - `WebSockets` by Markus Sattler (Links2004)

## Configuration

### Secrets

1. Copy `secrets.h.example` to `secrets.h`.
2. Edit `secrets.h` and enter your WiFi SSID, Password, and the API Key
   that clients (the Sensor Tester app) must present.

**Note:** `secrets.h` is excluded from Git to protect your credentials.

## How It Works

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts
  with a JSON packet containing the sensor type, IP, port, and current
  readings.
- **WebSocket server (port 9132):** Accepts `ws://` connections whose
  handshake carries a matching `X-Api-Key` header, then pushes a JSON
  reading every 250 ms. The app streams motion sensors instead of polling
  them, so there is no REST endpoint.
- **Fusion loop:** the MPU9250 is updated continuously from `loop()`;
  each push serves the latest cached roll/pitch/heading/g-force.

## Testing

```bash
# stream readings (pip install websockets)
python3 -c "
import asyncio, websockets
async def main():
    async with websockets.connect('ws://<esp32-ip>:9132',
                                  additional_headers={'X-Api-Key': 'your-sensor-api-key'}) as ws:
        for _ in range(5): print(await ws.recv())
asyncio.run(main())
"
```

Expected response:

```json
{"sensor":"IMU10DOF","host":"ESP32","temperature":23.1,"pressure":1009.4,"roll":-12.5,"pitch":4.2,"heading":271.0,"gforce":1.02}
```
