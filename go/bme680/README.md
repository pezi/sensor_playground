# BME680 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers with a BME680 I2C sensor (temperature, humidity,
pressure, IAQ). The Sensor Playground app discovers this node via UDP
broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header).

It is the Go counterpart of `../../python/bme680/` and speaks the exact
same Wi-Fi protocol. **BLE is not supported in this port** — setting
`"transport": "ble"` prints a warning and falls back to Wi-Fi; use the
Python node (or the Rust port in `../../rust/bme680/`) for BLE.

The BME680 driver is self-contained (`bme680.go` + `i2c.go`): a compact
port of the [Pimoroni Python driver](https://github.com/pimoroni/bme680-python)
using the same integer compensation formulas, with Bosch-datasheet fixes
for signed gas calibration and sample validity. The only dependency is
`golang.org/x/sys`.

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

Install Go (the Debian/Raspberry Pi OS `apt` version is usually too old;
use the official tarball — pick `arm64` or `armv6l` to match
`dpkg --print-architecture`):

```bash
curl -LO https://go.dev/dl/go1.24.5.linux-arm64.tar.gz
sudo rm -rf /usr/local/go && sudo tar -C /usr/local -xzf go1.24.5.linux-arm64.tar.gz
echo 'export PATH=$PATH:/usr/local/go/bin' >> ~/.profile && source ~/.profile
```

Then build the node (all Go nodes share one module with the common
transport package in [`../common/`](../common); build from this folder,
or cross-compile — see below):

```bash
go build -o sensor_node_bme680 .
cp config.example.json config.json
# Edit config.json: set api_key and the i2c_bus for your board
```

### Cross-compiling from another machine

Go cross-compiles without any extra toolchain — build on a desktop and
copy only the binary to the board:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_bme680 .   # Pi 3/4/5 (64-bit OS)
GOOS=linux GOARCH=arm GOARM=6 go build -o sensor_node_bme680 .   # 32-bit / Pi Zero
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
sensor hardware — it then serves plausible generated readings. Useful for
testing the app against a node on any machine (also works on macOS/Windows).

## SSL Certificates

Generate a self-signed certificate for HTTPS:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

The generated `cert.pem` and `key.pem` are referenced in `config.json`
and excluded from Git. They are resolved relative to the working
directory; `config.json` is looked up in the working directory first,
then next to the binary.

## Usage

```bash
./sensor_node_bme680
```

> Only one Sensor Playground node can run per board at a time — the
> Python, Go, Node.js and Rust nodes all share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

`go test` runs the driver's compensation math against 200 golden values.
Temperature, pressure, humidity and heater values come from the Pimoroni
reference; gas values correct its signed range-switch-error decoding. The
suite also covers concurrent reads and invalid gas handling.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-bme680-go.service`:

```ini
[Unit]
Description=Sensor Playground BME680 Node (Go)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-bme680-go
ExecStart=/home/pi/sensor-bme680-go/sensor_node_bme680
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-bme680-go
sudo systemctl start sensor-playground-bme680-go
```

## IAQ Calculation

The BME680 IAQ score is computed using a rolling-baseline algorithm
(ported from [dart_periphery](https://pub.dev/packages/dart_periphery)).
It maintains a window of 50 gas resistance readings to establish a
baseline, then scores gas (75%) and humidity (25%) relative to their
baselines. The score stabilizes after approximately 50 readings.

Rounding note: Go rounds halves away from zero while Python rounds
half-to-even; a reading can differ in the last decimal on exact halves,
which is far below the sensor's own tolerance.
