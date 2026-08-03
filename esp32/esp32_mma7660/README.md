# ESP32 MMA7660 Accelerometer Node for Sensor Tester

This Arduino project implements the Sensor Tester
[Sensor Interface](../../../docs/sensor.md) on an ESP32 with a
[Grove 3-Axis Digital Accelerometer ±1.5g](https://wiki.seeedstudio.com/Grove-3-Axis_Digital_Accelerometer-1.5g/)
(MMA7660FC) via I2C.

> The raw axes are converted into **roll** and **pitch** angles plus the
> total acceleration magnitude (**g-force**, ~1 g at rest). With its ±1.5 g
> range and 6-bit resolution the MMA7660 suits orientation/tilt sensing,
> not precise motion tracking.

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
- **Grove 3-Axis Digital Accelerometer ±1.5g** (MMA7660FC at `0x4C`)

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
   - `Grove 3-Axis Digital Accelerometer(±1.5g)` by Seeed Studio
     (provides `MMA7660.h`)
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
{"sensor":"MMA7660","host":"ESP32","roll":3.1,"pitch":-1.7,"gforce":0.99}
```
