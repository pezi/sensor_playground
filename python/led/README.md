# LED Node for Sensor Tester (Python)

An **actuator** node: the Sensor Tester app switches an LED on and off, and the
node reports the resulting state back. An optional push button wired to the
same node toggles the same LED locally, so the app follows changes it did not
cause.

Over Wi-Fi both directions travel on one **WebSocket** (`ws://`, port 9132,
`X-Api-Key` checked on the handshake), discovered via UDP broadcast on port
9133. Over BLE the node advertises the Sensor Tester GATT service instead: the
state arrives as a notification and the command as a write.

It is the Python/SoC counterpart of `../../esp32/esp32_led/` and supports the
same two transports (Wi-Fi and BLE).

> **The only two-way node.** Every other node either produces data (the
> sensors) or only consumes it (the SSD1306 display). This one does both, which
> is what lets a physical button press show up in the app.

The node owns the LED state; the app renders what the node last reported rather
than what it asked for, so a command that never arrived cannot leave the app
showing a lit LED that is in fact dark.

## Protocol

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"led": true}` | switch on |
| app → node | `{"led": false}` | switch off |
| app → node | `{"toggle": true}` | flip |
| node → app | `{"led": true\|false}` | current state — sent once right after connect and after every change, whatever caused it |

Over BLE the state notify carries the same JSON, but a command is a short
binary write instead (BLE writes are binary-safe):

| Packet | Meaning |
|--------|---------|
| `0x01 0x00` | switch off |
| `0x01 0x01` | switch on |
| `0x02` | flip |

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
it then prints each switch instead of driving a pin (works with both
transports). The virtual node has no button, so the app is the only thing that
switches it. Useful for testing the app on any machine.

## Hardware

- A Grove/BakeBit LED — https://wiki.seeedstudio.com/Grove-LED_Socket_Kit/
- *Optional:* a Grove/BakeBit button —
  https://wiki.seeedstudio.com/Grove-Button/

Set `"button_pin": null` for an LED-only node.

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `led_pin` | `5` | Pin driving the LED (BCM number for `"gpio"`, port number for `"hat"`). |
| `led_active_low` | `false` | `true` if the LED lights when the pin is driven LOW (some breakout boards). |
| `button_pin` | `6` | Pin of the local push button, or `null` for an LED-only node. |
| `button_active_low` | `true` | `true` if the button pulls the line LOW when pressed (Grove/BakeBit button). |

## Wiring: hat or direct GPIO

The `interface` config value selects how the pins are driven:

| `interface` | How | Use for |
|-------------|-----|---------|
| `"gpio"` | Raspberry Pi GPIOs via [gpiozero](https://gpiozero.readthedocs.io/). `led_pin` / `button_pin` are **BCM** numbers. | Direct wiring, **and the Seeed Grove Base Hat** — its digital ports are wired straight to the Pi. |
| `"hat"` | An Arduino-based hat over I2C, via the sibling [`extension_hat`](../extension_hat/) helper. The pins are hat port numbers, `hat_type` is `"nano"` or `"grovePlus"`. | FriendlyARM NanoHat Hub, Seeed GrovePi+. |

> The Grove Base Hat (`GroveBaseHat`) is **ADC-only** and has no digital
> command, so its digital ports use `"gpio"`, not `"hat"`.

On the `"gpio"` path the button's internal pull resistor is enabled to match
`button_active_low` (pull-up for `true`, pull-down for `false`), so the idle
level is defined. The Arduino hats expose only a plain INPUT, so an
`button_active_low: false` button needs an **external** pull-down there.

## Setup

```bash
python3 -m venv --system-site-packages venv
source venv/bin/activate
pip install -r requirements.txt    # installs both backends; you only need one

cp config.example.json config.json
# Edit: api_key, interface, led_pin, button_pin (and hat_type for "hat")
```

For `interface: "hat"` enable I2C (`sudo raspi-config` > Interface Options >
I2C).

### Why `--system-site-packages`

`interface: "gpio"` needs it. gpiozero is only a front-end and picks a *pin
factory* backend at runtime; on current Raspberry Pi OS that backend is
`lgpio`, shipped as the **system** package `python3-lgpio` (preinstalled).
A plain `python3 -m venv` hides it, and gpiozero then falls back through
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
Use `interface: "hat"` there.

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

The current state prints on connect. Type `{"led": true}` and press enter — the
LED lights and the node echoes `{"led": true}`. Pressing the physical button
prints a line too.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-led.service`:

```ini
[Unit]
Description=Sensor Tester LED Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-led
ExecStart=/home/pi/sensor-led/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-led
sudo systemctl start sensor-tester-led
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
