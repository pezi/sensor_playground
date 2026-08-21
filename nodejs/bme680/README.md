# BME680 Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface on
single-board computers with a BME680 I2C sensor (temperature, humidity,
pressure, IAQ). The Sensor Playground app discovers this node via UDP
broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header).

It is the Node.js counterpart of `../../python/bme680/` and speaks the
exact same Wi-Fi protocol. **BLE is not supported in this port** —
setting `"transport": "ble"` prints a warning and falls back to Wi-Fi;
use the Python node (or the Rust port in `../../rust/bme680/`) for BLE.

The BME680 driver is vendored (`bme680_driver.js`): a compact port of the
[Pimoroni Python driver](https://github.com/pimoroni/bme680-python) using
the same integer compensation formulas (BigInt — JavaScript's 32-bit
bitwise operators would corrupt them), with Bosch-datasheet fixes for
signed gas calibration and sample validity.
(The `bme680-sensor` npm package is abandoned; its current release is an
empty deprecation stub.) The only dependency is `i2c-bus`, installed as
an *optional* dependency so that emulation mode works on machines where
the native module cannot build.

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

Install Node.js ≥ 18 (Raspberry Pi OS Bookworm's `apt` version 18 works;
newer via [NodeSource](https://github.com/nodesource/distributions)):

```bash
curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
sudo apt install -y nodejs build-essential python3
```

(`build-essential` and `python3` are needed once to compile the `i2c-bus`
native module.)

Then set up the node. The shared [`../common/`](../common) folder must
be deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json
# Edit config.json: set api_key and the i2c_bus for your board
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
sensor hardware — it then serves plausible generated readings. Useful for
testing the app against a node on any machine (also works on
macOS/Windows, where `i2c-bus` is skipped as an optional dependency).

## SSL Certificates

Generate a self-signed certificate for HTTPS:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

The generated `cert.pem` and `key.pem` are referenced in `config.json`
and excluded from Git. They are resolved relative to the working
directory; `config.json` lives next to `sensor_node.js`.

## Usage

```bash
node sensor_node.js
```

> Only one Sensor Playground node can run per board at a time — the
> Python, Go, Node.js and Rust nodes all share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

`npm test` runs the vendored driver's compensation math against 200
golden values. Temperature, pressure, humidity and heater values come
from the Pimoroni reference; gas values correct its signed
range-switch-error decoding. It also checks read serialization and that
invalid gas samples do not alter the IAQ baseline.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-bme680-node.service`:

```ini
[Unit]
Description=Sensor Playground BME680 Node (Node.js)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-bme680-node
ExecStart=/usr/bin/node sensor_node.js
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-bme680-node
sudo systemctl start sensor-playground-bme680-node
```

## IAQ Calculation

The BME680 IAQ score is computed using a rolling-baseline algorithm
(ported from [dart_periphery](https://pub.dev/packages/dart_periphery)).
It maintains a window of 50 gas resistance readings to establish a
baseline, then scores gas (75%) and humidity (25%) relative to their
baselines. The score stabilizes after approximately 50 readings.
