# VL53L0X Sensor Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a VL53L0X I2C sensor (distance (time of flight)).
It is a *push* node: instead of serving readings over REST it pushes
one JSON message per event over a **WebSocket** (`ws://`, port 9132,
`X-Api-Key` checked on the handshake). The app discovers it via UDP
broadcast on port 9133. Over BLE the node advertises the Sensor Tester
GATT service instead and sends each event as a notification.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_vl53l0x/` and supports the same two transports (Wi-Fi
and BLE).

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

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

### Wiring (I2C)

| Pi Pin     | Sensor Pin |
|------------|------------|
| 3.3V (Pin 1) | VCC     |
| GND (Pin 6)  | GND     |
| GPIO 2 (Pin 3) | SDA   |
| GPIO 3 (Pin 5) | SCL   |

Enable I2C on the Pi:

```bash
sudo raspi-config   # Interface Options > I2C > Enable
```

## Software Setup

The Adafruit CircuitPython libraries pull in `Adafruit-Blinka`, which builds
C extensions (`RPi.GPIO`, `rpi_ws281x`) from source. Unlike the pure `smbus2`
sensor nodes, this one needs the Python development headers and a compiler
installed first, otherwise `pip install` fails with `Python.h: No such file
or directory`:

```bash
sudo apt-get update
sudo apt-get install -y python3-dev build-essential
```

```bash
# Create virtual environment
python3 -m venv --system-site-packages venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Copy and edit configuration
cp config.example.json config.json
# Edit config.json: set api_key and the i2c_bus for your board
```

### Why `--system-site-packages`

The Adafruit driver runs on **Adafruit-Blinka**, which depends on the
GPIO backend `lgpio`. On current Raspberry Pi OS (Debian 13, Python
3.13) piwheels has no prebuilt `lgpio` wheel, so inside a plain venv
pip builds it from source — which fails without swig:

```
swig -python -o lgpio_wrap.c lgpio.i
error: command 'swig' failed: No such file or directory
```

Raspberry Pi OS already ships the backend as the **system** packages
`python3-lgpio` and `python3-rpi-lgpio`; a `--system-site-packages`
venv lets pip see them and skip the build entirely. An existing venv
can be converted in place — set `include-system-site-packages = true`
in `venv/pyvenv.cfg`, then re-run `pip install -r requirements.txt`.

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

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-vl53l0x.service`:

```ini
[Unit]
Description=Sensor Tester VL53L0X Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-vl53l0x
ExecStart=/home/pi/sensor-vl53l0x/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-vl53l0x
sudo systemctl start sensor-tester-vl53l0x
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.

## Notes

The driver uses Adafruit Blinka (CircuitPython), which talks to the
board's default I2C bus — the `i2c_bus` config value is ignored on
single-bus boards.
