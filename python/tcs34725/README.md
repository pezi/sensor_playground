# TCS34725 Sensor Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a TCS34725 I2C sensor (RGB color, color temperature, illuminance).
Over Wi-Fi the Sensor Tester app discovers this node via UDP broadcast
(port 9133) and polls it for data over HTTPS (port 9132, `X-Api-Key`
header); over BLE the node advertises the Sensor Tester GATT service
instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_tcs34725/` and supports the same two transports (Wi-Fi
and BLE).

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

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — HTTPS REST server on port 9132 plus UDP discovery on
  port 9133. Requires the SSL certificates below.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No SSL certificates and no UDP discovery; BLE advertising
  is the discovery. Only one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the sensor
hardware — it then serves plausible generated readings (works with both
transports). Useful for testing the app against a node on any machine.

### SSL Certificates (Wi-Fi transport only)

Generate a self-signed certificate for HTTPS:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorTester"
```

The generated `cert.pem` and `key.pem` are referenced in `config.json`
and excluded from Git.

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-tcs34725.service`:

```ini
[Unit]
Description=Sensor Tester TCS34725 Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-tcs34725
ExecStart=/home/pi/sensor-tcs34725/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-tcs34725
sudo systemctl start sensor-tester-tcs34725
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.

## Notes

The driver uses Adafruit Blinka (CircuitPython), which talks to the
board's default I2C bus — the `i2c_bus` config value is ignored on
single-bus boards.
