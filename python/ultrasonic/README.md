# Grove Ultrasonic Ranger Node for Sensor Playground (Python)

This Python script implements the Sensor Playground sensor interface on
single-board computers with a
[Grove Ultrasonic Ranger](https://wiki.seeedstudio.com/Grove-Ultrasonic_Ranger/)
(40 kHz sonar, 2 cm – 3.5 m). It is a *push* node like the VL53L0X: it
measures continuously and pushes one JSON message over a **WebSocket**
(`ws://`, port 9132, `X-Api-Key` checked on the handshake) whenever the
distance changes, or at least once per second as a heartbeat. The app
discovers it via UDP broadcast on port 9133 and opens the same live
distance screen as for the VL53L0X. Over BLE the node advertises the
Sensor Playground GATT service instead and sends each measurement as a
notification.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_ultrasonic/` and supports the same two transports
(Wi-Fi and BLE).

> **Single SIG pin.** Unlike the common HC-SR04 with separate TRIG/ECHO
> pins, the Grove ranger multiplexes both on one wire: the node drives a
> trigger pulse, switches the pin to input and times the echo pulse the
> module answers with. The echo is timed via lgpio *alerts*, whose edge
> timestamps come from the kernel rather than from Python — a Python
> polling loop would add milliseconds of jitter, and one millisecond of
> pulse error is 17 cm of distance error. There is no extension-hat
> option: the Arduino-based hats are polled over I2C and cannot time the
> echo.

## Protocol

| Message | Meaning |
|---------|---------|
| `{"distance": 234}` | distance in millimeters |
| `{"distance": null}` | no echo / target out of range |

## Supported Platforms

| Board | Notes |
|-------|-------|
| Raspberry Pi | `gpio_chip: 0`; the SIG pin is a plain GPIO |

### Wiring

| Pi Pin | Grove Pin |
|--------|-----------|
| 5V (Pin 2) or 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 4 (Pin 7, `sig_pin` in config) | SIG (yellow) |

> The module runs on 3.3–5V. When powering from 5V, prefer 3.3V if you
> want to be conservative about the SIG level on the Pi's input.

## Software Setup

```bash
# Create virtual environment — --system-site-packages lets it see the
# system python3-lgpio, which cannot be pip-installed
sudo apt install python3-lgpio
python3 -m venv --system-site-packages venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Copy and edit configuration
cp config.example.json config.json
# Edit config.json: set api_key and the sig_pin your ranger is wired to
```

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `api_key` | — | Shared key the app must present (min. 8 characters) |
| `hostname` | `""` | Name shown in the app; empty = system hostname |
| `sig_pin` | `4` | BCM GPIO of the SIG line |
| `gpio_chip` | `0` | gpiochip number (`0` on Raspberry Pi) |
| `transport` | `wifi` | `wifi` (WebSocket + UDP discovery) or `ble` |
| `emulation` | `false` | Serve generated readings without hardware |

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

Set `"emulation": true` in `config.json` to run the node without the
ranger hardware — it then simulates a target sweeping between 200 and
2000 mm (works with both transports). Useful for testing the app against
a node on any machine.

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Wave a hand in front of the ranger — `{"distance": ...}` messages track
it; point it into open space and `{"distance": null}` appears.

## Running as a Service (optional)

```ini
# /etc/systemd/system/sensor-playground-ultrasonic.service
[Unit]
Description=Sensor Playground ultrasonic ranger node
After=network-online.target

[Service]
WorkingDirectory=/home/pi/ultrasonic
ExecStart=/home/pi/ultrasonic/venv/bin/python3 sensor_node.py
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

For the BLE transport add `After=bluetooth.target` and run as
`User=root`.
