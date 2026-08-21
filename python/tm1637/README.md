# TM1637 Clock Node for Sensor Playground (Python)

An **actuator** node: the Sensor Playground app shows a digital clock and syncs
`hh:mm` to a Grove 4-Digit Display (TM1637), and the node reports the state it
is actually displaying back. Once a time is set the node keeps the clock
running on its own — it advances the minute and blinks the colon locally, so
the display stays a working clock even when the app is away.

Over Wi-Fi both directions travel on one **WebSocket** (`ws://`, port 9132,
`X-Api-Key` checked on the handshake), discovered via UDP broadcast on port
9133. Over BLE the node advertises the Sensor Playground GATT service instead: the
state arrives as a notification and the commands as writes.

It is the Python/SoC counterpart of `../../esp32/esp32_tm1637/` and supports
the same two transports (Wi-Fi and BLE).

The node owns the displayed state; the app renders what the node last reported
rather than what it asked for, so a command that never arrived cannot leave
the app showing a time the display does not. Until the first time arrives the
display shows `--:--`.

## Protocol

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"time": "HH:MM"}` | set the displayed time (24-hour; invalid values are ignored) |
| app → node | `{"brightness": 0..7}` | set the brightness (clamped; 0 is dimmest but still lit) |
| node → app | `{"time": "HH:MM", "brightness": 3}` | current state — sent once right after connect, after every accepted command, and on each minute rollover |
| node → app | `{"time": null, "brightness": 3}` | no time has been set yet (display shows `--:--`) |

The colon blink is local only and never generates a message, so state traffic
stays at one message per minute.

Over BLE the state notify carries the same JSON, but a command is a short
binary write instead (BLE writes are binary-safe):

| Packet | Meaning |
|--------|---------|
| `0x01 <hh> <mm>` | set the time (rejected unless `hh` ≤ 23 and `mm` ≤ 59) |
| `0x02 <0..7>` | set the brightness (clamped) |
| `0x03` | re-notify the current state |

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket server on port 9132 plus UDP discovery on
  port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketch
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery. Only
  one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without any hardware —
it then prints the displayed time instead of driving a panel (works with both
transports), on time/brightness changes and minute rollovers only. Useful for
testing the app on any machine.

## Hardware

- A Grove 4-Digit Display (TM1637) —
  https://wiki.seeedstudio.com/Grove-4-Digit_Display/

The TM1637 speaks a proprietary two-wire protocol (start/stop conditions like
I2C, but LSB-first and without addresses) and has no minimum clock speed, so
the node bit-bangs it directly on two Pi GPIOs — plain Python pin toggling is
fast enough.

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `clk_pin` | `5` | BCM number of the GPIO driving the display clock line. |
| `dio_pin` | `6` | BCM number of the GPIO driving the display data line. |
| `brightness` | `3` | Brightness at start (0 dimmest … 7 brightest; 0 is still lit — the TM1637 has no off level below it). |

## Wiring: direct GPIO only

`interface` must be `"gpio"`: the pins are driven directly via
[gpiozero](https://gpiozero.readthedocs.io/), `clk_pin` / `dio_pin` are
**BCM** numbers. This also covers the **Seeed Grove Base Hat** — its digital
ports are wired straight to the Pi.

> The Arduino-based extension hats (`"hat"` — NanoHat Hub, GrovePi+) are
> **not supported** for this node: one display frame needs ~50 line
> transitions and each hat `digital_write` is a full I2C transaction, far too
> slow for a display bus. The node refuses to start with a clear message.

| Pi Pin | Grove Pin |
|--------|-----------|
| 3.3V or 5V | VCC |
| GND | GND |
| `clk_pin` (default BCM 5) | CLK (yellow) |
| `dio_pin` (default BCM 6) | DIO (white) |

The TM1637 acknowledges every byte by pulling DIO low; the node clocks
through that ACK slot without reading it (gpiozero output pins cannot be
released mid-transfer), which the chip tolerates.

## Setup

```bash
python3 -m venv --system-site-packages venv
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key, clk_pin, dio_pin
```

### Why `--system-site-packages`

gpiozero needs it. gpiozero is only a front-end and picks a *pin factory*
backend at runtime; on current Raspberry Pi OS that backend is `lgpio`,
shipped as the **system** package `python3-lgpio` (preinstalled). A plain
`python3 -m venv` hides it, and gpiozero then falls back through
`lgpio` → `RPi.GPIO` → `pigpio` to its experimental native factory, which
drives the sysfs interface `/sys/class/gpio` — removed in Debian 13 (trixie).
The node dies on the first `DigitalOutputDevice(...)`:

```
PinFactoryFallback: Falling back from lgpio: No module named 'lgpio'
...
OSError: [Errno 22] Invalid argument   # writing /sys/class/gpio/export
```

An existing venv can be converted in place — set
`include-system-site-packages = true` in `venv/pyvenv.cfg`, no rebuild needed.
`pip install lgpio` is *not* an alternative: the PyPI package builds its
C extension with swig, which is not installed by default.

If `python3-lgpio` is missing: `sudo apt install python3-lgpio`.

On a **non-Raspberry-Pi** board none of this applies: gpiozero identifies the
board from `/proc/device-tree` and refuses to start anywhere else
(`PinUnknownPi: unable to locate Pi revision`), even with `lgpio` installed.
Use the ESP32 sketch there instead.

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

The current state prints on connect (`{"time": null, "brightness": 3}` on a
fresh start). Type `{"time": "12:34"}` and press enter — the display shows
12:34 with a blinking colon and the node echoes the new state. A minute later
it echoes `12:35` on its own.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-tm1637.service`:

```ini
[Unit]
Description=Sensor Playground TM1637 Clock Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-tm1637
ExecStart=/home/pi/sensor-tm1637/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-tm1637
sudo systemctl start sensor-playground-tm1637
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
