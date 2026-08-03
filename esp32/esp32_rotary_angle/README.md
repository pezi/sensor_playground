# ESP32 Rotary Angle Sensor Node for Sensor Tester

Reads a [Grove Rotary Angle Sensor](https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/)
— a 10 kΩ potentiometer with 300° of mechanical travel — on an ADC pin and
reports the knob position to the Sensor Tester app, which draws it as a needle
on a dial.

Unlike the light sensor (also analog, but polled over HTTPS every few seconds)
this is a **push** node: the app's needle tracks the knob, so a reading that is
seconds old is useless. The node samples continuously and sends a message
whenever the knob has moved past the deadband.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "ROTARY"`, host, ip, `port: 9132`). Like the
  other push nodes it advertises identity only — the position arrives on the
  WebSocket.
- **WebSocket (port 9132, `ws://`):** The client presents the shared
  `X-Api-Key` header on the handshake. The node then pushes one text message
  per movement (and the current position once, right after connect):

  ```json
  {"adc":2048,"adcMax":4095,"angle":150.1,"angleMax":300.0}
  ```

| Key | Meaning |
|-----|---------|
| `adc` | Raw ADC count, `0 … adcMax`. |
| `adcMax` | Full-scale count of **this** board — see below. |
| `angle` | Shaft position in degrees, `adc / adcMax × angleMax`. |
| `angleMax` | Mechanical travel of the knob (300° for the Grove sensor). |

The same payload is what the BLE transport notifies.

> This node uses plaintext `ws://` (not `wss://`), consistent with the other
> push nodes (gesture, distance, digital contact).

## Why `adcMax` is in every message

The converter's width belongs to the board doing the reading, not to the knob,
and the boards this project supports do not agree:

| Board | Resolution | Range |
|-------|-----------|-------|
| **ESP32 (this sketch)** | 12-bit | `0 – 4095` |
| Seeed Grove Base Hat | 12-bit | `0 – 4095` |
| FriendlyARM NanoHat Hub (BakeBit) | 10-bit | `0 – 1023` |
| Seeed GrovePi+ | 10-bit | `0 – 1023` |

A bare `512` is therefore ambiguous — half travel on a NanoHat, an eighth on an
ESP32. Sending the full-scale value alongside the count lets the app scale its
dial to whichever node it is talking to instead of assuming one of them, and
`angle`/`angleMax` carry the same position in degrees so the app never needs to
know the knob's mechanical travel either.

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server (9132) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, then subscribes to the data characteristic. Each movement is one notify. |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- Grove Rotary Angle Sensor

### Wiring

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 3.3V | VCC |
| GND | GND |
| `ROTARY_PIN` (default GPIO 34) | SIG (yellow) |

> Power the sensor from **3.3 V**, not 5 V. It is a plain voltage divider, so at
> 5 V the wiper swings past what the ESP32's ADC can read and the top of the
> travel saturates at `4095` well before the end stop.
>
> `ROTARY_PIN` must be an **ADC1** pin (GPIO 32–39): ADC2 stops working the
> moment the Wi-Fi radio is on. GPIO 34 is input-only, which suits a sensor.

## Configuration

Set these at the top of the sketch:

| Constant | Default | Meaning |
|----------|---------|---------|
| `ROTARY_PIN` | `34` | ADC1 pin the signal wire is on. |
| `ADC_BITS` | `12` | ADC resolution; also fixes `ADC_MAX` at `4095`. |
| `ANGLE_MAX` | `300.0` | Mechanical travel of the knob, in degrees. |
| `ADC_DEADBAND` | `24` | How far the count must move before a message goes out (~0.6 % of full scale). Raise it if a resting knob still chatters. |
| `MIN_PUBLISH_INTERVAL_MS` | `40` | Rate ceiling — 25 messages/s at most. |
| `OVERSAMPLE` | `8` | Conversions averaged per sample, to damp ADC noise. |

Then copy `secrets.h.example` to `secrets.h` and fill in WiFi + API key.
`secrets.h` is excluded from Git. This node has no HTTPS endpoint, so no TLS
certificate is needed.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `WebSockets` by Markus Sattler (links2004/arduinoWebSockets)

   No sensor library — the potentiometer is read with `analogRead`.

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

The current position prints on connect; turning the knob prints a JSON line per
movement. Turn it to both end stops and check that `adc` reaches `0` and `4095`
— if the top saturates early, the sensor is on 5 V instead of 3.3 V.

## See also

`../../python/rotary_angle/` — the same node for Raspberry Pi & co.
