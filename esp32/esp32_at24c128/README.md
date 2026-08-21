# ESP32 AT24C128 EEPROM Node for Sensor Playground

An **actuator** node: the Sensor Playground app stores a short text in an AT24C128
serial EEPROM (128 Kbit / 16 KB, I2C) and reads it back at any time. The text
survives power cycles of both the node and the app.

> **A two-way node.** Like the LED and the TM1637 clock this node both
> consumes commands and reports state. The node is the single source of
> truth: after a write it reads the chip back and reports the *stored* text,
> so a failed write cannot leave the app showing a text the chip never held.

## EEPROM layout

The text lives at the start of the chip:

| Offset | Content |
|--------|---------|
| 0–1 | magic `'S' 'P'` |
| 2–3 | text length in bytes (u16 big-endian, max 512) |
| 4–… | UTF-8 text |

A chip without the magic (e.g. factory-fresh, all `0xFF`) reads as an empty
text. The app limits the text to 128 characters; the 512-byte region fits
that even when every character encodes to 4 UTF-8 bytes.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "AT24C128"`, host, ip, `port: 9132`). The app
  recognises the type and opens the EEPROM screen.
- **WebSocket (port 9132, `ws://`):** The client presents the shared
  `X-Api-Key` header on the handshake. Both directions are text messages:

  | Direction | Message | Meaning |
  |-----------|---------|---------|
  | app → node | `{"write": "Hello"}` | store the text on the chip |
  | app → node | `{"read": true}` | re-read the chip and push |
  | node → app | `{"text": "Hello"}` | stored text — sent once right after connect and after every write/read, always read back from the chip |

> This node uses plaintext `ws://` (not `wss://`), consistent with the other
> push nodes (gesture, distance, digital contact).

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server (9132) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, subscribes to the data characteristic for the stored text and writes commands to the command characteristic. |

Over BLE the text notify carries the same `{"text": …}` JSON, but a write is
staged in offset-addressed binary chunks (like the SSD1306 bitmap), because a
text may not fit in a single ATT write:

| Packet | Meaning |
|--------|---------|
| `0x01 <offset:u16 BE> <bytes…>` | stage a chunk of UTF-8 text |
| `0x02 <length:u16 BE>` | store the staged text (refused unless exactly `length` bytes are staged) |
| `0x03` | re-read the chip and push |

Chunks must arrive contiguously; staging offset 0 restarts the transfer, so a
dropped packet leaves the EEPROM untouched rather than storing a torn text.

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- An AT24C128 EEPROM breakout (or the bare chip) —
  https://ww1.microchip.com/downloads/en/DeviceDoc/doc0670.pdf

### Wiring

| ESP32 Pin | AT24C128 Pin |
|-----------|--------------|
| 3.3V | VCC |
| GND | GND (and A0–A2, WP for address 0x50, writable) |
| GPIO 21 (`SDA_PIN`) | SDA |
| GPIO 22 (`SCL_PIN`) | SCL |

Most breakout boards carry pull-up resistors on SDA/SCL; add 4.7 kΩ pull-ups
to 3.3V when wiring a bare chip.

## Configuration

Set these at the top of the sketch:

| Define | Default | Meaning |
|--------|---------|---------|
| `EEPROM_ADDR` | `0x50` | I2C address (0x50–0x57, set by the A0–A2 pins). |
| `SDA_PIN` / `SCL_PIN` | `21` / `22` | I2C pins. |

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

The stored text prints on connect. Type `{"write": "Hello EEPROM"}` and press
enter — the node stores it and echoes `{"text": "Hello EEPROM"}` read back
from the chip. Power-cycle the board, reconnect, and the same text prints
again.

## See also

`../../python/at24c128/` — the same node for Raspberry Pi & co.
