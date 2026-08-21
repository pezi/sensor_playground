# SHT11 Sensor Node for Sensor Playground (Python)

This Python script implements the Sensor Playground sensor interface on
single-board computers with a Sensirion SHT11 sensor (temperature,
humidity) — the classic SHT1x family. Over Wi-Fi the Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header); over BLE the node advertises
the Sensor Playground GATT service instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_sht11/` and supports the same two transports (Wi-Fi
and BLE).

The SHT10, SHT11 and SHT15 differ only in calibration accuracy (±0.5 /
±0.4 / ±0.3 °C typical) and speak the same proprietary **two-wire
protocol** (SCK + bidirectional DATA). It resembles I2C but is *not*
I2C — the sensor cannot share an I2C bus with other devices. Set
`sensor_name` in `config.json` to the chip on your board so the app
shows the right name.

> The protocol is bit-banged directly on two GPIOs via `lgpio`: the bus
> is fully master-clocked with no minimum speed, so Python's pace is
> harmless — the sensor simply waits between clock edges (unlike the
> DHT11, whose reply timing a Python loop cannot follow). There is no
> extension-hat option. The sensor must not be measured more than ~10%
> of the time or it heats itself; the node reads at most every two
> seconds and serves the cached values, and a failed read serves the
> last good reading for up to 30 seconds.

## Supported Platforms

| Board | Notes |
|-------|-------|
| Raspberry Pi | `gpio_chip: 0`; the two pins are plain GPIOs |

### Wiring (two-wire, NOT I2C)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VDD |
| GND (Pin 6) | GND |
| GPIO 4 (Pin 7, `data_pin` in config) | DATA |
| GPIO 5 (Pin 29, `sck_pin` in config) | SCK |

> The DATA line needs a pull-up resistor (~10 kΩ to 3.3V); most breakout
> boards carry one, and the node additionally enables the Pi's internal
> pull-up. The temperature conversion constant assumes a 3.3V supply.

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
# Edit config.json: set api_key and the pins your sensor is wired to
```

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `api_key` | — | Shared key the app must present (min. 8 characters) |
| `hostname` | `""` | Name shown in the app; empty = system hostname |
| `data_pin` | `4` | BCM GPIO of the bidirectional DATA line |
| `sck_pin` | `5` | BCM GPIO of the clock line |
| `gpio_chip` | `0` | gpiochip number (`0` on Raspberry Pi) |
| `sensor_name` | `"SHT11"` | `SHT10`, `SHT11` or `SHT15`, per your board |
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
hardware — it then serves plausible generated readings (works with both
transports). Useful for testing the app against a node on any machine.

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
{"sensor":"SHT11","host":"raspberrypi","temperature":22.4,"humidity":46.1}
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-sht11.service`:

```ini
[Unit]
Description=Sensor Playground SHT11 Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-sht11
ExecStart=/home/pi/sensor-sht11/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-sht11
sudo systemctl start sensor-playground-sht11
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
