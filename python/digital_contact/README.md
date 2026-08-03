# Digital Contact Sensor Node for Sensor Tester (Python)

One generic **push** node for the simple two-state Grove/BakeBit digital
sensors. It polls a debounced input and tells the Sensor Tester app whenever
the state changes — over Wi-Fi as one JSON message per event on a
**WebSocket** (`ws://`, port 9132, `X-Api-Key` checked on the handshake),
discovered via UDP broadcast on port 9133. Over BLE the node advertises the
Sensor Tester GATT service instead and sends each event as a notification.

It is the Python/SoC counterpart of
`../../esp32/esp32_digital_contact/` and supports the same two transports
(Wi-Fi and BLE).

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket push server on port 9132 plus UDP
  discovery on port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery. Only
  one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the sensor
hardware — it then pushes plausible generated readings (works with both
transports). Useful for testing the app against a node on any machine.

## Supported sensors

Set `sensor_name` and `active_low` in `config.json` for your sensor:

| Sensor | `sensor_name` | `active_low` | Wiki |
|--------|---------------|--------------|------|
| Button | `"BUTTON"` | `true` | https://wiki.seeedstudio.com/Grove-Button/ |
| Hall sensor | `"HALL"` | `true` | https://wiki.seeedstudio.com/Grove-Hall_Sensor/ |
| Magnetic switch | `"MAGSWITCH"` | `false` | https://wiki.seeedstudio.com/Grove-Magnetic_Switch/ |
| PIR motion | `"PIR"` | `false` | https://wiki.seeedstudio.com/Grove-PIR_Motion_Sensor/ |
| Vibration (SW-420) | `"VIBRATION"` | `true` | https://wiki.seeedstudio.com/Grove-Vibration_Sensor_SW-420/ |
| Line Finder | `"LINEFINDER"` | `false` * | https://wiki.seeedstudio.com/Grove-Line_Finder/ |

`active_low: true` means the sensor pulls the signal line LOW when active.

> \* The Line Finder is an infrared reflectance detector (TCRT5000 plus
> comparator): it sees a dark line against a bright surface a few millimetres
> below it — the classic line-following robot sensor. Its output polarity
> differs between board revisions, and the on-board potentiometer sets the
> black/white *threshold* rather than the direction. Hold the sensor over the
> line and check the app: if it reads "Line detected" over the bright surface
> instead, flip `active_low`.

The node enables the matching internal pull resistor so the idle state is
defined: a pull-up for `active_low: true` (idles HIGH) and a pull-down for
`active_low: false` (idles LOW). The pull-down matters for the reed-based
magnetic switch, which only connects the line to VCC while the magnet holds
it shut and floats otherwise. On the `"hat"` interface only — the Arduino
hats have no internal pull-down — an `active_low: false` reed sensor needs an
**external** pull-down resistor (e.g. 10 kΩ to GND); the direct-GPIO path
needs nothing extra.

## Wiring: hat or direct GPIO

The `interface` config value selects how the input is read:

| `interface` | How | Use for |
|-------------|-----|---------|
| `"gpio"` | A Raspberry Pi GPIO via [gpiozero](https://gpiozero.readthedocs.io/). `pin` is the **BCM** number. | Direct wiring, **and the Seeed Grove Base Hat** — its digital ports are wired straight to the Pi. |
| `"hat"` | An Arduino-based hat over I2C, via the sibling [`extension_hat`](../extension_hat/) helper. `pin` is the hat port number, `hat_type` is `"nano"` or `"grovePlus"`. | FriendlyARM NanoHat Hub, Seeed GrovePi+. |

> The Grove Base Hat (`GroveBaseHat`) is **ADC-only** and has no digital-read
> command, so its digital ports use `"gpio"`, not `"hat"`.

## Setup

```bash
python3 -m venv --system-site-packages venv
source venv/bin/activate
pip install -r requirements.txt    # installs both backends; you only need one

cp config.example.json config.json
# Edit: api_key, sensor_name, active_low, interface, pin (and hat_type for "hat")
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
The node dies on the first `DigitalInputDevice(...)`:

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
Use `interface: "hat"` there — verified on a NanoPi NEO 2 (Armbian) with a
NanoHat Hub.

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

Trigger the sensor; each change prints as a JSON line
(`{"active": true}` / `{"active": false}`). On connect the node first sends
the current state.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-contact.service`:

```ini
[Unit]
Description=Sensor Tester Digital Contact Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-digital_contact
ExecStart=/home/pi/sensor-digital_contact/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-contact
sudo systemctl start sensor-tester-contact
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
