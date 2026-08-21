# ESP32 Relay Node for Sensor Playground

> [!WARNING]
> ### ⚡ Mains voltage kills — read this before wiring the load side
>
> These modules are rated **10 A at 250 VAC**. The contact side is built to
> switch **230 V / 120 V mains**, which is lethal to touch and will start a
> fire if it is wired or enclosed badly.
>
> - **De-energize first.** Pull the plug and switch off the breaker before you
>   touch a screw terminal — "relay off" in the app is *not* de-energized.
> - **Treat every contact as live while the board is powered**, whatever the
>   app shows. A welded contact, a mis-numbered channel, a crash or a stray
>   command can leave a channel closed while the app says open.
> - **Enclose the module before powering it.** Bare screw terminals and 230 V
>   do not belong on a desk.
> - **Keep the two sides apart.** Mains on the contact side only — never
>   running alongside the low-voltage wiring to the ESP32, and never bridging
>   the isolation slot in the board.
> - **This is not a safety device.** A relay is not an isolator, an emergency
>   stop or a lockout. Never rely on it to make anything safe to work on.
> - **Fixed mains wiring is a licensed trade in most countries.** If any of
>   this is unfamiliar, switch a low-voltage load instead — a 12 V lamp or fan
>   behaves identically in the app.

An **actuator** node: the Sensor Playground app switches the channels of a
Grove SPDT relay module — 1, 2 or 4 of them — and the node reports the
resulting state of every channel back.

> **The node names its own size.** How many channels the board carries is
> configured here and travels with the discovery reply and with every state
> message, so the app draws exactly one button per relay. The same sketch and
> the same app screen serve all three module sizes.

The node owns the relay state; the app renders whatever the node last reported
rather than what it asked for, so a command that never arrived cannot leave the
app showing a closed contact that is in fact open.

## Protocol

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts with a
  JSON identity packet (`type: "RELAY"`, host, ip, `port: 9132`, `channels`).
  The app recognises the type and opens the relay screen.
- **WebSocket (port 9132, `ws://`):** The client presents the shared
  `X-Api-Key` header on the handshake. Both directions are text messages:

  | Direction | Message | Meaning |
  |-----------|---------|---------|
  | app → node | `{"ch": 0, "on": true}` | switch channel 0 on (channels are zero-based on the wire; the app labels them from 1) |
  | app → node | `{"ch": 0, "on": false}` | switch channel 0 off |
  | app → node | `{"ch": 0, "toggle": true}` | flip channel 0 |
  | app → node | `{"all": false}` | switch every channel off |
  | node → app | `{"channels": 2, "relay": [true,false]}` | state of the whole board — sent once right after connect and after every change |

> This node uses plaintext `ws://` (not `wss://`), consistent with the other
> push nodes (LED, gesture, distance, digital contact).

## Transport (compile-time switch)

| `ACTIVE_TRANSPORT` | Behaviour |
|--------------------|-----------|
| `TRANSPORT_WIFI` | WebSocket server (9132) + UDP discovery (9133). |
| `TRANSPORT_BLE`  | BLE GATT service; the app writes the API key to the auth characteristic, subscribes to the data characteristic for the state and writes commands to the command characteristic. |

Over BLE the state notify carries the same `{"channels": …, "relay": […]}`
JSON, but a command is a short binary write instead of a JSON document (BLE
writes are binary-safe):

| Packet | Meaning |
|--------|---------|
| `0x01 <ch> 0x00` | switch channel `<ch>` off |
| `0x01 <ch> 0x01` | switch channel `<ch>` on |
| `0x02 <ch>` | flip channel `<ch>` |
| `0x03 0x00` / `0x03 0x01` | switch every channel at once |

## Hardware

- **ESP32** (e.g., NodeMCU, DevKit v1)
- A relay module:
  - Grove Relay, 1 channel — https://wiki.seeedstudio.com/Grove-Relay/
  - Grove 2-Channel SPDT Relay — https://wiki.seeedstudio.com/Grove-2-Channel_SPDT_Relay/
  - Grove 4-Channel SPDT Relay — https://wiki.seeedstudio.com/Grove-4-Channel_SPDT_Relay/
  - [SunFounder 4 Channel 5V Relay Module](https://www.sunfounder.com/products/4channel-relay-shield)
    and the many clones of that board

> Everything except the **Grove** 4-channel module is switched over plain
> digital pins — one per channel, 1 to 8 of them, and the length of
> `RELAY_PINS` is the channel count. The Grove 4-channel module is the odd one
> out: an **I2C device** (factory address `0x11`) whose on-board MCU drives the
> coils. The sketch covers both — pick the wiring with `RELAY_INTERFACE`.
>
> The coils draw about 90 mA each, far more than a GPIO can source: the
> modules carry their own driver transistor and want **5 V on VCC**. Feed them
> from the board's 5 V rail (or a separate supply sharing GND), not from 3.3 V.

### Wiring — digital modules (1 to 8 channels)

| ESP32 Pin | Grove Pin | SunFounder-style board |
|-----------|-----------|------------------------|
| 5V | VCC (red) | VCC |
| GND | GND (black) | GND |
| `RELAY_PINS[0]` (default GPIO 5) | SIG1 (yellow) | IN1 |
| `RELAY_PINS[1]` (default GPIO 18) | SIG2 (white, 2-channel module only) | IN2 |
| `RELAY_PINS[2]` | — | IN3 |
| `RELAY_PINS[3]` | — | IN4 |

On the Grove modules, driving a signal HIGH energizes that coil, moving COM
from NC to NO — leave `RELAY_ACTIVE_LOW` at `false`.

> **Check the trigger polarity on non-Grove boards.** The common 4-channel
> "relay module" boards are usually **active LOW**: a channel closes when its
> input is pulled LOW, so they need `RELAY_ACTIVE_LOW = true`. The sketch
> opens every channel at boot, so if the relays click *closed* on reset (or
> the status LEDs light), the flag is the wrong way round — and the load is
> energized while the app shows "off".
>
> SunFounder rates that board at 5 V with 15-20 mA of drive current per
> channel and contacts good for 10 A at 250 VAC / 30 VDC. If a channel will
> not switch reliably from the ESP32's 3.3 V logic, give the coils their own
> 5 V supply (remove the VCC/JD-VCC jumper if the board has one, feed JD-VCC
> from 5 V, share GND) so the input stage only has to see the logic level.

### Wiring — I2C module (4 channels)

| ESP32 Pin | Grove Pin |
|-----------|-----------|
| 5V | VCC (red) |
| GND | GND (black) |
| GPIO 21 | SDA (white) |
| GPIO 22 | SCL (yellow) |

## Configuration

Set these at the top of the sketch:

| Define | Default | Meaning |
|--------|---------|---------|
| `RELAY_INTERFACE` | `RELAY_IFACE_GPIO` | `RELAY_IFACE_GPIO` for the 1- and 2-channel modules, `RELAY_IFACE_I2C` for the 4-channel one. |
| `RELAY_PINS` | `{5, 18}` | GPIO interface: one GPIO per channel, in channel order — the first entry is "Relay 1" in the app. **The length of this list is the channel count**, so a 1-channel board lists a single pin. |
| `RELAY_ACTIVE_LOW` | `false` | `true` if a coil is energized when its pin is driven LOW. The Grove modules are active HIGH. |
| `RELAY_I2C_ADDRESS` | `0x11` | I2C interface: the module's address (`0x12` on some batches). |
| `RELAY_I2C_CHANNELS` | `4` | I2C interface: how many channels the board carries. |

Then copy `secrets.h.example` to `secrets.h` and fill in WiFi + API key.
`secrets.h` is excluded from Git. This node has no HTTPS endpoint, so no TLS
certificate is needed.

## Software Requirements

1. **Arduino IDE** or **VSCode with PlatformIO**
2. **Board Support**: ESP32 by Espressif
3. **Libraries** (Library Manager):
   - `ArduinoJson` by Benoit Blanchon
   - `WebSockets` by Markus Sattler (links2004/arduinoWebSockets)

The I2C module needs no extra library: the sketch writes its one-byte channel
bitmask command (`0x10 <mask>`, bit 0 = channel 1) directly with `Wire`, the
same bytes Seeed's `Multi_Channel_Relay` library sends.

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

The current state prints on connect. Type `{"ch": 0, "on": true}` and press
enter — the first relay clicks and the node echoes the state of the whole
board. `{"all": false}` opens every contact again.

## Safety

**These modules switch mains-level loads (10 A at 250 VAC / 30 VDC).** Read the
warning at the top of this file before you wire anything to the contact side.

In short: de-energize the load and the circuit before touching a terminal,
treat the contacts as live whenever the board is powered, keep mains wiring on
the contact side and away from the low-voltage side to the ESP32, and
enclose the module before powering it. The node opens every channel on a clean
shutdown, but a crash, a power cut or a pulled cable leaves the contacts where
they happened to be — so never treat the relay as an isolator, an emergency
stop or a lockout for work on the load.

If mains wiring is not something you are comfortable and qualified to do, use a
low-voltage load instead: the node behaves identically with a 12 V lamp, and
in many countries fixed mains work is legally reserved for electricians.

## See also

`../../python/relay/` — the same node for Raspberry Pi & co.
