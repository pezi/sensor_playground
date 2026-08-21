# ESP32 TM1637 Clock Node for Sensor Playground

An **actuator** node: the Sensor Playground app shows a digital clock and syncs
`hh:mm` to a Grove 4-Digit Display (TM1637), and the node reports the state it
is actually displaying back. Once a time is set the node keeps the clock
running on its own — it advances the minute and blinks the colon locally, so
the display stays a working clock even when the app is away.

The node owns the displayed state; the app renders whatever the node last
reported rather than what it asked for, so a command that never arrived cannot
leave the app showing a time the display does not.

Until the first time arrives the display shows `--:--`.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "TM1637"`, host, ip, `port: 9132`). The app
  recognises the type and opens the clock screen.
- **WebSocket (port 9132, `ws://`):** The client presents the shared
  `X-Api-Key` header on the handshake. Both directions are text messages:

  | Direction | Message | Meaning |
  |-----------|---------|---------|
  | app → node | `{"time": "HH:MM"}` | set the displayed time (24-hour; invalid values are ignored) |
  | app → node | `{"brightness": 0..7}` | set the brightness (clamped; 0 is dimmest but still lit) |
  | node → app | `{"time": "HH:MM", "brightness": 3}` | current state — sent once right after connect, after every accepted command, and on each minute rollover |
  | node → app | `{"time": null, "brightness": 3}` | no time has been set yet (display shows `--:--`) |

  The colon blink is local only and never generates a message, so state
  traffic stays at one message per minute.

> This node uses plaintext `ws://` (not `wss://`), consistent with the other
> push nodes (gesture, distance, digital contact, LED).

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server (9132) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, subscribes to the data characteristic for the state and writes commands to the command characteristic. |

Over BLE the state notify carries the same JSON, but a command is a short
binary write instead of a JSON document (BLE writes are binary-safe):

| Packet | Meaning |
|--------|---------|
| `0x01 <hh> <mm>` | set the time (rejected unless `hh` ≤ 23 and `mm` ≤ 59) |
| `0x02 <0..7>` | set the brightness (clamped) |
| `0x03` | re-notify the current state |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- A Grove 4-Digit Display (TM1637) —
  https://wiki.seeedstudio.com/Grove-4-Digit_Display/

The TM1637 speaks a proprietary two-wire protocol (start/stop conditions like
I2C, but LSB-first and without addresses). The sketch bit-bangs it directly on
two GPIOs — no display library is needed.

### Wiring

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 3.3V or 5V | VCC |
| GND | GND |
| `TM_CLK_PIN` (default GPIO 5) | CLK (yellow) |
| `TM_DIO_PIN` (default GPIO 4) | DIO (white) |

## Configuration

Set these at the top of the sketch:

| Define | Default | Meaning |
|--------|---------|---------|
| `TM_CLK_PIN` | `5` | GPIO driving the display clock line. |
| `TM_DIO_PIN` | `4` | GPIO driving the display data line. |
| `DEFAULT_BRIGHTNESS` | `3` | Brightness at boot (0 dimmest … 7 brightest; 0 is still lit — the TM1637 has no off level below it). |

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

The current state prints on connect (`{"time":null,"brightness":3}` on a
fresh boot). Type `{"time": "12:34"}` and press enter — the display shows
12:34 with a blinking colon and the node echoes the new state. A minute later
it echoes `12:35` on its own.

## See also

`../../python/tm1637/` — the same node for Raspberry Pi & co.
