# ESP32 Digital Contact Sensor Node for Sensor Tester

One generic **push** node for the simple two-state Grove/BakeBit digital
sensors — it reads a debounced GPIO and tells the Sensor Tester app whenever
the state changes.

> **Ignore the LED.** The original dart_periphery hat examples toggle a local
> LED when the sensor reacts. This node does not — it reports the state to the
> app instead.

## Supported sensors

Set the three config defines at the top of the sketch for your sensor:

| Sensor | `SENSOR_NAME` | `ACTIVE_LOW` | Wiki |
|--------|---------------|--------------|------|
| Button | `"BUTTON"` | `true` | https://wiki.seeedstudio.com/Grove-Button/ |
| Hall sensor | `"HALL"` | `true` | https://wiki.seeedstudio.com/Grove-Hall_Sensor/ |
| Magnetic switch | `"MAGSWITCH"` | `false` | https://wiki.seeedstudio.com/Grove-Magnetic_Switch/ |
| PIR motion | `"PIR"` | `false` | https://wiki.seeedstudio.com/Grove-PIR_Motion_Sensor/ |
| Vibration (SW-420) | `"VIBRATION"` | `true` | https://wiki.seeedstudio.com/Grove-Vibration_Sensor_SW-420/ |
| Line Finder | `"LINEFINDER"` | `false` * | https://wiki.seeedstudio.com/Grove-Line_Finder/ |

`ACTIVE_LOW = true` means the sensor pulls the signal line LOW when active (the
internal pull-up is enabled for these); `false` means it drives the line HIGH.
`INPUT_PIN` is the GPIO the Grove signal (yellow) wire is on.

> \* The Line Finder is an infrared reflectance detector (TCRT5000 plus
> comparator): it sees a dark line against a bright surface a few millimetres
> below it — the classic line-following robot sensor. Its output polarity
> differs between board revisions, and the on-board potentiometer sets the
> black/white *threshold* rather than the direction. Hold the sensor over the
> line and check the app: if it reads "Line detected" over the bright surface
> instead, flip `ACTIVE_LOW`.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: SENSOR_NAME`, host, ip, `port: 9132`). The app
  recognises the type and opens the generic trigger screen.
- **WebSocket (port 9132, `ws://`):** The client presents the shared
  `X-Api-Key` header on the handshake. The node then pushes one text message
  per state change (and the current state once, right after connect):

  ```json
  {"active": true}
  {"active": false}
  ```

The `{"active": …}` payload is identical on the BLE transport, where each
change arrives as one notify.

> This node uses plaintext `ws://` (not `wss://`), consistent with the other
> push nodes (gesture, distance).

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server (9132) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, then subscribes to the data characteristic. Each state change is one notify. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- One of the Grove digital sensors above

### Wiring

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 3.3V or 5V | VCC (per sensor) |
| GND | GND |
| `INPUT_PIN` (default GPIO 4) | SIG (yellow) |

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `WebSockets` by Markus Sattler (links2004/arduinoWebSockets)

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
2. Set `SENSOR_NAME`, `ACTIVE_LOW` and `INPUT_PIN` for your sensor.

`secrets.h` is excluded from Git. This node has no HTTPS endpoint, so no TLS
certificate is needed.

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://<esp32-ip>:9132/
```

Trigger the sensor; each change prints as a JSON line.
