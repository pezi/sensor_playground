# ESP32 LED Node for Sensor Tester

An **actuator** node: the Sensor Tester app switches an LED on the ESP32 on and
off, and the node reports the resulting state back. An optional push button
wired to the same node toggles the same LED locally, so the app follows changes
it did not cause.

> **The only two-way node.** Every other node either produces data (the
> sensors) or only consumes it (the SSD1306 display). This one does both, which
> is what lets a physical button press show up in the app.

The node owns the LED state; the app renders whatever the node last reported
rather than what it asked for, so a command that never arrived cannot leave the
app showing a lit LED that is in fact dark.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "LED"`, host, ip, `port: 9132`). The app
  recognises the type and opens the LED screen.
- **WebSocket (port 9132, `ws://`):** The client presents the shared
  `X-Api-Key` header on the handshake. Both directions are text messages:

  | Direction | Message | Meaning |
  |-----------|---------|---------|
  | app → node | `{"led": true}` | switch on |
  | app → node | `{"led": false}` | switch off |
  | app → node | `{"toggle": true}` | flip |
  | node → app | `{"led": true\|false}` | current state — sent once right after connect and on every change, whatever caused it |

> This node uses plaintext `ws://` (not `wss://`), consistent with the other
> push nodes (gesture, distance, digital contact).

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server (9132) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, subscribes to the data characteristic for the state and writes commands to the command characteristic. |

Over BLE the state notify carries the same `{"led": …}` JSON, but a command is
a short binary write instead of a JSON document (BLE writes are binary-safe):

| Packet | Meaning |
|--------|---------|
| `0x01 0x00` | switch off |
| `0x01 0x01` | switch on |
| `0x02` | flip |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- A Grove/BakeBit LED — https://wiki.seeedstudio.com/Grove-LED_Socket_Kit/
- *Optional:* a Grove/BakeBit button —
  https://wiki.seeedstudio.com/Grove-Button/

### Wiring

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 3.3V or 5V | VCC (both modules) |
| GND | GND (both modules) |
| `LED_PIN` (default GPIO 5) | LED SIG (yellow) |
| `BUTTON_PIN` (default GPIO 4) | Button SIG (yellow) |

## Configuration

Set these at the top of the sketch:

| Define | Default | Meaning |
|--------|---------|---------|
| `LED_PIN` | `5` | GPIO driving the LED. |
| `LED_ACTIVE_LOW` | `false` | `true` if the LED lights when the pin is driven LOW (some breakout boards and several dev boards' built-in LEDs). |
| `BUTTON_PIN` | `4` | GPIO of the local push button, or `-1` for an LED-only node. |
| `BUTTON_ACTIVE_LOW` | `true` | `true` if the button pulls the line LOW when pressed (Grove/BakeBit button). The matching internal pull-up / pull-down is enabled automatically, so the idle level is defined. |

`BUTTON_PIN` must be a GPIO with internal pulls — the input-only GPIO34-39 have
none.

Then copy `secrets.h.example` to `secrets.h` and fill in WiFi + API key.
`secrets.h` is excluded from Git. This node has no HTTPS endpoint, so no TLS
certificate is needed.

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

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://<esp32-ip>:9132/
```

The current state prints on connect. Type `{"led": true}` and press enter — the
LED lights and the node echoes the new state. Pressing the physical button
prints a line too.

## See also

`../../python/led/` — the same node for Raspberry Pi & co.
