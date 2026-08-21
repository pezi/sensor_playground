# ESP32 Ultrasonic Ranger Node for Sensor Playground

A **push** node: the ESP32 measures distance continuously with a
[Grove Ultrasonic Ranger](https://wiki.seeedstudio.com/Grove-Ultrasonic_Ranger/)
(40 kHz sonar, 2 cm – 3.5 m) and pushes one JSON message whenever the
distance changes, or at least once per second as a heartbeat. The app
opens the same live distance screen as for the VL53L0X time-of-flight
node.

> **Single SIG pin.** Unlike the common HC-SR04 with separate TRIG/ECHO
> pins, the Grove ranger multiplexes both on one wire: the sketch drives
> a 10 µs trigger pulse, switches the pin to input and times the echo
> pulse the module answers with. The pulse width divided by twice the
> speed of sound is the distance. A missing echo (no target, or beyond
> 3.5 m) is published as `null`, which the app shows as out of range.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts
  with a JSON identity packet (`type: "ULTRASONIC"`, host, ip,
  `port: 9132`). The app recognises the type and opens the distance
  screen.
- **WebSocket (port 9132, `ws://`):** The client presents the shared
  `X-Api-Key` header on the handshake, then receives one text message
  per publish:

  | Message | Meaning |
  |---------|---------|
  | `{"distance": 234}` | distance in millimeters |
  | `{"distance": null}` | no echo / target out of range |

> This node uses plaintext `ws://` (not `wss://`), consistent with the
> other push nodes (gesture, VL53L0X distance, digital contact).

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server (9132) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic and subscribes to the data characteristic — each measurement arrives as one notify carrying the same JSON. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- A Grove Ultrasonic Ranger —
  https://wiki.seeedstudio.com/Grove-Ultrasonic_Ranger/

### Wiring

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 5V (or 3.3V) | VCC (red) |
| GND | GND (black) |
| `SIG_PIN` (default GPIO 4) | SIG (yellow) |

> The module runs on 3.3–5V. At 5V supply the SIG echo level is still
> ESP32-safe on the Grove module (the signal is driven by the module's
> MCU at its logic level); when in doubt, power it from 3.3V.

## Configuration

Set these at the top of the sketch:

| Define | Default | Meaning |
|--------|---------|---------|
| `SIG_PIN` | `4` | GPIO the SIG line is wired to. |
| `MIN_DELTA_MM` | `5` | Smallest change worth a publish. |
| `MEASURE_INTERVAL_MS` | `100` | Pause between pings (echo die-down). |

Then copy `secrets.h.example` to `secrets.h` and fill in WiFi + API key.
`secrets.h` is excluded from Git. This node has no HTTPS endpoint, so no
TLS certificate is needed.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `WebSockets` by Markus Sattler (links2004/arduinoWebSockets)
   - (no sensor library — the trigger/echo timing lives in the sketch)

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

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://<esp32-ip>:9132/
```

Wave a hand in front of the ranger — `{"distance": ...}` messages track
it; point it into open space and `{"distance": null}` appears.

## See also

`../../python/ultrasonic/` — the same node for Raspberry Pi & co.
