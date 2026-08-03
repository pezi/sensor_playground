# ESP32 PAJ7620 Gesture Sensor Node for Sensor Tester

This Arduino project drives a **Grove Gesture sensor (PAJ7620U2)** on an ESP32
and reports detected hand gestures to the Sensor Tester app.

> **Why push instead of REST?** The environment sensors (BME680, SCD30, …)
> expose a value you can poll at any time. A gesture sensor has no such value —
> it only produces data at the *instant* a gesture happens. Polling would
> either miss gestures or waste bandwidth, so this node pushes each gesture
> the moment it is detected.

## Transport (compile-time switch)

Set `ACTIVE_TRANSPORT` near the top of the sketch:

| Value            | Behaviour |
|------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server on port 9132 (`ws://`, `X-Api-Key` validated on the handshake) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service. The app scans for the Sensor Tester service UUID, writes the API key to the auth characteristic, then subscribes to the data characteristic; each gesture arrives as one **notify**. |

The `{"gesture":"…"}` payload is identical on both transports. The BLE GATT
UUIDs are the shared Sensor Tester contract (`d1a51b00-000{1,2,3}-…`, see the
sketch) and must match the app's `BleUuids`.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "PAJ7620"`, host, ip, `port: 9132`). The app
  recognises the `PAJ7620` type and opens the gesture screen instead of the
  REST live-data screen.
- **WebSocket (port 9132, `ws://`):** On connect the client must present the
  shared `X-Api-Key` header (validated during the WebSocket handshake — a wrong
  key is rejected). Thereafter the node pushes one text message per gesture:

  ```json
  {"gesture":"forward"}
  ```

  The `gesture` value is one of the app's `Gesture` enum names:
  `forward, forwardBackward, backward, backwardForward, right, rightLeft,
  left, leftRight, up, upDown, down, downUp, clockwise, antiClockwise, wave`.

> This node uses plaintext `ws://` (not `wss://`). It is intended for trusted
> LAN use, consistent with the rest of the project's self-signed-cert model.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **Grove Gesture sensor (PAJ7620U2)**

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
   - `WebSockets` by Markus Sattler (links2004/arduinoWebSockets)
   - `Gesture PAJ7620` by Seeed Studio

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

> This node has no HTTPS endpoint, so no TLS certificate is needed — the
> `SERVER_CERT` / `SERVER_KEY` fields used by the other sketches are omitted.

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://<esp32-ip>:9132/
```

Then wave your hand in front of the sensor; each gesture prints as a JSON line.
