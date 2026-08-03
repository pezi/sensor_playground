# ESP32 MPU6050 IMU Node for Sensor Tester

This Arduino project implements the Sensor Tester
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with an MPU6050
6-axis IMU (accelerometer + gyroscope) via I2C.

> The accelerometer axes are converted into **roll** and **pitch** angles
> plus the total acceleration magnitude (**g-force**, ~1 g at rest); the
> on-die **temperature** is reported as well (it runs slightly warm).
> The MPU6050 has no magnetometer, so no compass heading is available.

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
- **MPU6050** breakout (e.g., GY-521) at `0x68` (`0x69` with AD0 high)

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
   - `Adafruit MPU6050` (pulls in `Adafruit Unified Sensor`)
   - `WebSockets` by Markus Sattler (Links2004) — `TRANSPORT_WIFI` only

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

**Note:** `secrets.h` is excluded from Git to protect your credentials.

## How It Works

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts
  with a JSON packet containing the sensor type, IP, port, and current
  readings.
- **WebSocket server (port 9132):** Accepts `ws://` connections whose
  handshake carries a matching `X-Api-Key` header, then pushes a JSON
  reading every 250 ms. The app streams accelerometers instead of polling
  them, so there is no REST endpoint.

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
{"sensor":"MPU6050","host":"ESP32","temperature":28.4,"roll":1.8,"pitch":-0.6,"gforce":1.01}
```
