# DHT11 Sensor Node for Sensor Playground (Python)

This Python script implements the Sensor Playground sensor interface on
single-board computers with a DHT11 sensor (temperature, humidity) — the
blue
[Grove Temperature & Humidity Sensor](https://wiki.seeedstudio.com/Grove-TemperatureAndHumidity_Sensor/)
module. Over Wi-Fi the Sensor Playground app discovers this node via UDP
broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header); over BLE the node advertises the Sensor Playground GATT
service instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_dht11/` and supports the same two transports (Wi-Fi
and BLE).

The DHT22 — the white Grove "Pro" module — speaks the same single-wire
protocol with better resolution and range; it has its own node in
[`../dht22/`](../dht22/). Setting `sensor_name` to `DHT22` here still
picks the matching driver, for configurations that predate that folder.

> The single-wire protocol is timing-critical (26–70 µs pulses), so the
> pin is read through `adafruit-circuitpython-dht` — it ships a small
> compiled pulse reader that a Python loop cannot replace. There is no
> extension-hat option: the Arduino-based hats are polled over I2C and
> cannot follow those pulses either. Reads occasionally fail even on a
> healthy sensor; the node retries and serves the last good reading for
> up to 30 seconds, so a single failed read does not surface as an
> error.

## Supported Platforms

| Board | Notes |
|-------|-------|
| Raspberry Pi | any model; the signal pin is a plain GPIO |

### Wiring (single-wire digital)

| Pi Pin | Grove Pin |
|--------|-----------|
| 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 4 (Pin 7, `gpio_pin` in config) | SIG (yellow) |

> The Grove module already carries the required pull-up resistor on the
> signal line; add a 10 kΩ pull-up to 3.3V when wiring a bare DHT11.

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
# Edit config.json: set api_key and the gpio_pin your sensor is wired to
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

Raspberry Pi OS already ships the backend as the **system** package
`python3-lgpio`; a `--system-site-packages` venv lets pip see it and
skip the build entirely. An existing venv can be converted in place —
set `include-system-site-packages = true` in `venv/pyvenv.cfg`, then
re-run `pip install -r requirements.txt`.

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `api_key` | — | Shared key the app must present (min. 8 characters) |
| `hostname` | `""` | Name shown in the app; empty = system hostname |
| `gpio_pin` | `4` | BCM GPIO number the signal line is wired to |
| `sensor_name` | `"DHT11"` | Type the node advertises to the app (`DHT22` selects the Pro module's driver; prefer `../dht22/`) |
| `transport` | `wifi` | `wifi` (HTTPS REST + UDP discovery) or `ble` |
| `emulation` | `false` | Serve generated readings without hardware |

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
hardware — it then serves plausible generated readings quantised to the
DHT11's whole-degree resolution (works with both transports). Useful for
testing the app against a node on any machine.

### SSL Certificates (Wi-Fi transport only)

Generate a self-signed certificate for HTTPS:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
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

Expected response:

```json
{"sensor":"DHT11","host":"raspberrypi","temperature":22.0,"humidity":46.0}
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-dht11.service`:

```ini
[Unit]
Description=Sensor Playground DHT11 Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-dht11
ExecStart=/home/pi/sensor-dht11/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-dht11
sudo systemctl start sensor-playground-dht11
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
