# Relay Node for Sensor Playground (Python)

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
>   running alongside the low-voltage wiring to the Raspberry Pi, and never
>   bridging the isolation slot in the board.
> - **This is not a safety device.** A relay is not an isolator, an emergency
>   stop or a lockout. Never rely on it to make anything safe to work on.
> - **Fixed mains wiring is a licensed trade in most countries.** If any of
>   this is unfamiliar, switch a low-voltage load instead — a 12 V lamp or fan
>   behaves identically in the app.

An **actuator** node: the Sensor Playground app switches the channels of a
Grove SPDT relay module — 1, 2 or 4 of them — and the node reports the
resulting state of every channel back. Over Wi-Fi the app discovers this node
via UDP broadcast (port 9133) and talks to it over a WebSocket (port 9132,
`X-Api-Key` header); over BLE the node advertises the Sensor Playground GATT
service instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_relay/` and supports the same two transports (Wi-Fi and
BLE).

> **The node names its own size.** How many channels the board carries comes
> from `config.json` — the length of `relay_pins`, or `channels` for the I2C
> module — and travels with the discovery reply and with every state message,
> so the app draws exactly one button per relay.

The node owns the relay state; the app renders whatever the node last reported
rather than what it asked for, so a command that never arrived cannot leave the
app showing a closed contact that is in fact open.

## Protocol

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"ch": 0, "on": true}` | switch channel 0 on (channels are zero-based on the wire; the app labels them from 1) |
| app → node | `{"ch": 0, "on": false}` | switch channel 0 off |
| app → node | `{"ch": 0, "toggle": true}` | flip channel 0 |
| app → node | `{"all": false}` | switch every channel off |
| node → app | `{"channels": 2, "relay": [true,false]}` | state of the whole board — sent once right after connect and after every change |

Over BLE the same JSON arrives as a notify, while a command is a short binary
write (BLE writes are binary-safe):

| Packet | Meaning |
|--------|---------|
| `0x01 <ch> 0x00` | switch channel `<ch>` off |
| `0x01 <ch> 0x01` | switch channel `<ch>` on |
| `0x02 <ch>` | flip channel `<ch>` |
| `0x03 0x00` / `0x03 0x01` | switch every channel at once |

## Supported Hardware

| Module | Channels | Interface |
|--------|----------|-----------|
| [Grove Relay](https://wiki.seeedstudio.com/Grove-Relay/) | 1 | one digital pin |
| [Grove 2-Channel SPDT Relay](https://wiki.seeedstudio.com/Grove-2-Channel_SPDT_Relay/) | 2 | one digital pin per channel |
| [Grove 4-Channel SPDT Relay](https://wiki.seeedstudio.com/Grove-4-Channel_SPDT_Relay/) | 4 | I2C (factory address `0x11`) |
| [SunFounder 4 Channel 5V Relay Module](https://www.sunfounder.com/products/4channel-relay-shield) | 4 | one digital pin per channel |

| Board | Notes |
|-------|-------|
| Raspberry Pi | any model; digital pins are plain GPIOs, the I2C module goes on bus 1 |
| NanoPi / GrovePi+ | the digital modules also work through an Arduino-based hat (`"interface": "hat"`) |

> The coils draw about 90 mA each, far more than a GPIO can source: the
> modules carry their own driver transistor and want **5 V on VCC**. Feed them
> from the Pi's 5 V pin (or a separate supply sharing GND), not from 3.3 V —
> the signal line is 3.3 V-tolerant either way.

### Wiring — digital modules (1 and 2 channels)

| Pi Pin | Grove Pin |
|--------|-----------|
| 5V (Pin 2) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 5 (Pin 29, first entry of `relay_pins`) | SIG1 (yellow) |
| GPIO 6 (Pin 31, second entry of `relay_pins`) | SIG2 (white, 2-channel module only) |

Driving a signal HIGH energizes that coil, moving COM from NC to NO.

### Wiring — 4-channel digital boards (SunFounder & co.)

Boards of the common "4 Channel 5V Relay Module" shape — the
[SunFounder shield](https://www.sunfounder.com/products/4channel-relay-shield)
and its many clones — are **not** I2C: they expose one input pin per channel,
so they use the same digital interface as the Grove 1- and 2-channel modules,
just with four pins:

| Pi Pin | Relay board |
|--------|-------------|
| 5V | VCC |
| GND | GND |
| GPIO 5 (Pin 29) — first entry of `relay_pins` | IN1 |
| GPIO 6 (Pin 31) — second entry | IN2 |
| GPIO 13 (Pin 33) — third entry | IN3 |
| GPIO 19 (Pin 35) — fourth entry | IN4 |

```json
{
  "interface": "gpio",
  "relay_pins": [5, 6, 13, 19],
  "relay_active_low": true
}
```

> **Check the trigger polarity.** Most of these boards are **active LOW** — a
> channel closes when its input is pulled LOW — while the Grove modules are
> active HIGH. The node opens every channel at startup, so if the relays click
> *closed* the moment it starts (or the LEDs come on), flip
> `relay_active_low`. Getting this backwards leaves the load energized while
> the app shows "off".

> **Supply.** SunFounder rates the board at 5 V with 15-20 mA of drive current
> per channel and contacts good for 10 A at 250 VAC / 30 VDC. Feed VCC from
> 5 V, not 3.3 V. If a channel will not switch reliably from 3.3 V logic, use
> the board's separate coil supply (remove the VCC/JD-VCC jumper if it has
> one, feed JD-VCC from 5 V and share GND) so the input stage only has to see
> the logic level.

### Wiring — I2C module (4 channels)

| Pi Pin | Grove Pin |
|--------|-----------|
| 5V (Pin 2) | VCC (red) |
| GND (Pin 6) | GND (black) |
| SDA (GPIO 2, Pin 3) | SDA (white) |
| SCL (GPIO 3, Pin 5) | SCL (yellow) |

Enable I2C once with `sudo raspi-config` → *Interface Options* → *I2C*, then
check the module answers: `i2cdetect -y 1` should show `11`.

## Software Setup

```bash
# Create virtual environment — --system-site-packages lets it see the
# system python3-lgpio, which cannot be pip-installed (see below)
python3 -m venv --system-site-packages venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Copy and edit configuration
cp config.example.json config.json
# Edit config.json: set api_key and the pins/interface your board uses
```

### Why `--system-site-packages`

`gpiozero` is only a front-end and needs a *pin factory* backend, which on
current Raspberry Pi OS is the system package `python3-lgpio`. Inside a plain
venv pip cannot install it (piwheels has no wheel, the source build needs
swig), and gpiozero then falls back to `/sys/class/gpio`, gone in Debian 13. A
`--system-site-packages` venv sees the system package instead. An existing venv
can be converted in place — set `include-system-site-packages = true` in
`venv/pyvenv.cfg`, then re-run `pip install -r requirements.txt`.

The I2C module needs no such backend, only `smbus2`.

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `api_key` | — | Shared key the app must present (min. 8 characters) |
| `hostname` | `""` | Name shown in the app; empty = system hostname |
| `sensor_name` | `"RELAY"` | Type the node advertises to the app |
| `interface` | `gpio` | `gpio` (Pi GPIOs), `hat` (NanoHat Hub / GrovePi+) or `i2c` (4-channel module) |
| `relay_pins` | `[5, 6]` | `gpio`/`hat`: one pin per channel, in channel order. **Its length is the channel count** — a 1-channel board lists one pin |
| `relay_active_low` | `false` | `true` if a coil is energized by a LOW signal. The Grove modules are active HIGH |
| `channels` | `4` | `i2c`: how many channels the board carries |
| `i2c_address` | `"0x11"` | `i2c`: the module's address (`"0x12"` on some batches) |
| `i2c_bus` | `1` | I2C bus number (`0` for the Arduino-based hats) |
| `hat_type` | `nano` | `hat`: `nano` (NanoHat Hub) or `grovePlus` (GrovePi+) |
| `transport` | `wifi` | `wifi` (WebSocket + UDP discovery) or `ble` |
| `emulation` | `false` | Switch virtual relays without hardware |

The node writes the module's own command byte directly (`0x10 <mask>`, bit 0 =
channel 1), so Seeed's `Multi_Channel_Relay` library is not needed.

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next to
this node folder.

- `"wifi"` (default) — WebSocket server on port 9132 plus UDP discovery on
  port 9133. No SSL certificates: like the other push nodes this one uses
  plaintext `ws://`.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketch (see
  `../common/README.md` for the GATT contract, BlueZ prerequisites and
  testing). BLE advertising is the discovery. Only one BLE node can run per
  board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without any relay
hardware — it prints every switch instead of driving coils, and still reports
the channel count your configuration describes, so the app shows the same
buttons it would in the field.

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://localhost:9132/
```

The current state prints on connect:

```json
{"channels": 2, "relay": [false, false]}
```

Type `{"ch": 0, "on": true}` and press enter — the first relay clicks and the
node echoes the state of the whole board. `{"all": false}` opens every contact
again.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-relay.service`:

```ini
[Unit]
Description=Sensor Playground Relay Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-relay
ExecStart=/home/pi/sensor-relay/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-relay
sudo systemctl start sensor-playground-relay
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.

## Safety

**These modules switch mains-level loads (10 A at 250 VAC / 30 VDC).** Read the
warning at the top of this file before you wire anything to the contact side.

In short: de-energize the load and the circuit before touching a terminal,
treat the contacts as live whenever the board is powered, keep mains wiring on
the contact side and away from the low-voltage side to the Raspberry Pi, and
enclose the module before powering it. The node opens every channel on a clean
shutdown, but a crash, a power cut or a pulled cable leaves the contacts where
they happened to be — so never treat the relay as an isolator, an emergency
stop or a lockout for work on the load.

If mains wiring is not something you are comfortable and qualified to do, use a
low-voltage load instead: the node behaves identically with a 12 V lamp, and
in many countries fixed mains work is legally reserved for electricians.

## See also

`../../esp32/esp32_relay/` — the same node for the ESP32.
