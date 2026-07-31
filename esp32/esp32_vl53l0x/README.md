# ESP32 VL53L0X Distance Sensor Node for Sensor Tester

This Arduino project drives a **VL53L0X time-of-flight distance sensor** on an
ESP32 and pushes measurements to the Sensor Tester app.

> **Why push instead of REST?** Like the gesture node, the distance node is
> event-driven: it measures continuously (~10 Hz) and pushes a message only
> when the distance actually changes, so the app reacts instantly without
> polling.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "VL53L0X"`, host, ip, `port: 9132`). The app
  recognises the `VL53L0X` type and opens the distance screen instead of the
  REST live-data screen.
- **Push payload** (WebSocket text message or BLE notify):

  ```json
  {"distance": 234}
  ```

  Distance is in **millimeters**; when no target is within range the node
  sends `{"distance": null}`. A message is pushed when the distance moves by
  ≥3 mm, the in/out-of-range state flips, or once per second as a heartbeat.

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server on port 9132 (`ws://`, `X-Api-Key` validated on the handshake) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Tester service UUID, writes the API key to the auth characteristic, then subscribes to the data characteristic; each measurement arrives as one **notify**. |

The payload is identical on both transports. The BLE GATT UUIDs are the
shared Sensor Tester contract (`d1a51b00-000{1,2,3}-…`, see the sketch) and
must match the app's `BleUuids`.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **VL53L0X** module (time-of-flight ranging, ~30–1200 mm)

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
   - `Adafruit_VL53L0X`
   - `WebSockets` by Markus Sattler (links2004/arduinoWebSockets) — Wi-Fi only

## Configuration

### Secrets

1. Copy `secrets.h.example` to `secrets.h`.
2. Edit `secrets.h` and enter your WiFi SSID, Password, and the API Key
   that clients (the Sensor Tester app) must present.

**Note:** `secrets.h` is excluded from Git to protect your credentials.

> This node has no HTTPS endpoint, so no TLS certificate is needed — the
> `SERVER_CERT` / `SERVER_KEY` fields used by the environment-sensor sketches
> are omitted.

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://<esp32-ip>:9132/
```

Then move a hand in front of the sensor; each change prints as a JSON line.
