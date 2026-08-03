# ESP32 SSD1306 Display Node for Sensor Tester

This Arduino project drives an **SSD1306 128×64 I2C OLED display** on an ESP32
and shows bitmaps pushed by the Sensor Tester app.

> **Why a WebSocket instead of REST?** The environment sensors (BME680,
> SCD30, …) expose values the app polls. A display has nothing to poll — the
> data flows the *other* way: the app pushes one command per action and the
> node draws it the moment it arrives. This is the reverse direction of the
> gesture/distance push nodes, over the same WebSocket transport.

## Transport

Selected at compile time via `ACTIVE_TRANSPORT`, like the other sketches:

- `TRANSPORT_WIFI` (default) — WebSocket on port 9132 + UDP discovery on 9133
- `TRANSPORT_BLE` — BLE GATT service; the app writes commands to a fourth
  characteristic (see *BLE protocol* below)

The BLE build uses ~85% of the default partition's program space. If you add
to the sketch and it no longer fits, switch to a larger app partition
(Tools > Partition Scheme > "Huge APP").

## Protocol (Wi-Fi)

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "SSD1306"`, host, ip, `port: 9132`). The app
  recognises the `SSD1306` type and opens the display screen instead of the
  REST live-data screen.
- **WebSocket (port 9132, `ws://`):** On connect the client must present the
  shared `X-Api-Key` header (validated during the WebSocket handshake — a
  wrong key is rejected). Thereafter the **app** pushes one text message per
  command:

  ```json
  {"image": "<base64>"}
  {"clear": true}
  ```

  The bitmap is 1024 bytes (128×64 pixels, 1 bit per pixel), packed
  horizontally row by row — 16 bytes per row, the MSB of each byte is the
  leftmost pixel. This matches the
  [dart_periphery SSD1306 example](https://github.com/pezi/dart_periphery/blob/main/example/i2c_ssd1306.dart)
  and the [image2cpp](https://javl.github.io/image2cpp/) "horizontal" byte
  orientation. The sketch transposes it into the SSD1306 native page format
  (8 pages × 128 column bytes, LSB on top) before writing it over I2C.

> This node uses plaintext `ws://` (not `wss://`). It is intended for trusted
> LAN use, consistent with the rest of the project's self-signed-cert model.

## Protocol (BLE)

The app scans for the service UUID, writes the API key to the auth
characteristic, then writes commands to the command characteristic:

| Attribute | UUID | Properties |
|-----------|------|------------|
| Service | `d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f` | advertised (device name `SSD1306`) |
| Data | `d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f` | READ, NOTIFY — always `{}`, see below |
| Auth | `d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f` | WRITE (plain API key) |
| Command | `d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f` | WRITE, WRITE NR |

A frame cannot travel as one message — an ATT write carries at most MTU-3
bytes — so the bitmap arrives as a series of chunks followed by a show
command:

| Packet | Effect |
|--------|--------|
| `0x01 <offset:u16 big-endian> <bytes…>` | Stage a chunk at `offset` |
| `0x02` | Draw the staged frame |
| `0x03` | Blank the display |

The chunks are sent raw rather than base64-encoded (BLE writes are
binary-safe), which keeps the transfer at 1024 bytes instead of the ~1.4 KB
the JSON form costs. They must arrive contiguously; writing offset 0 restarts
the frame, so a client that gives up mid-frame simply starts over. A `0x02`
on an incomplete frame is refused rather than drawn — otherwise a dropped
chunk would show as a band of stale pixels from the previous image.

The sketch requests a 517-byte MTU; at the 23-byte default a frame would take
~52 writes instead of ~3.

The data characteristic carries no readings (a display produces none) and
always reads `{}`. It exists so every Sensor Tester node exposes the same
characteristics and the app can connect with one code path.

Commands are ignored until the API key has been written, and a disconnect
clears both the authentication and any half-staged frame.

## Hardware Requirements

- **ESP32** (e.g., NodeMCU, DevKit v1)
- **SSD1306 128×64 OLED display** (I2C variant)

Most boards use I2C address `0x3C` (the sketch default); some are strapped to
`0x3D` — adjust `OLED_ADDRESS` in the sketch.

### Wiring (I2C)

| ESP32 Pin | Display Pin |
|-----------|-------------|
| 3.3V      | VCC         |
| GND       | GND         |
| GPIO 21   | SDA         |
| GPIO 22   | SCL         |

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (install via Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `WebSockets` by Markus Sattler (links2004/arduinoWebSockets) —
     `TRANSPORT_WIFI` only; BLE uses the ESP32 core's built-in stack

   No display library is needed — the SSD1306 is driven with raw I2C writes
   (a port of dart_periphery's `ssd1306.dart`).

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

Wi-Fi, with [`websocat`](https://github.com/vi/websocat):

```bash
echo '{"clear": true}' | \
  websocat -H='X-Api-Key: your-sensor-api-key' ws://<esp32-ip>:9132/
```

The display blanks; sending an `image` command draws the bitmap.

BLE, with a generic client such as nRF Connect: connect to `SSD1306`, write
the API key to `…0003…` as a UTF-8 string, then write `03` to `…0004…` to
blank the panel. Sending a whole bitmap by hand is impractical — use the app.
The serial monitor logs each accepted command and the reason for each
rejected one.
