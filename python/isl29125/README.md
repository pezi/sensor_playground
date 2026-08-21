# SparkFun ISL29125 RGB Light Sensor Node for Sensor Playground (Python)

This Python script reads a **SparkFun RGB Light Sensor breakout** with the
Intersil/Renesas **ISL29125** (I2C address `0x44`) and serves the measured
color and an approximate illuminance to the Sensor Playground app. Over Wi-Fi
the Sensor Playground app discovers this node via UDP broadcast (port 9133) and
polls it for data over HTTPS (port 9132, `X-Api-Key` header); over BLE the
node advertises the Sensor Playground GATT service instead.
https://www.sparkfun.com/sparkfun-rgb-light-sensor-isl29125.html

The chip measures the intensity of red, green and blue light while rejecting
infrared from light sources.

> **Pollable, not push.** The sensor produces continuous values, so it uses
> the same pollable transports as the environment sensors. The app shows an
> "Illuminance" card/chart and a live color swatch.

It is the Python/SoC counterpart of `../../esp32/esp32_isl29125/` and
supports the same two transports (Wi-Fi and BLE).

## Readings

| JSON key (REST) | Meaning |
|-----------------|---------|
| `red`/`green`/`blue` | measured color, normalized against the brightest channel (`0`-`255`); absent in complete darkness |
| `lux` | approximate illuminance derived from the green channel, whose spectral response resembles the human eye (10K lux range, 16-bit) |

Unlike the TCS34725 the ISL29125 has **no clear channel**, so no color
temperature is reported.

### Wiring

| Pi Pin | Breakout Pin |
|--------|--------------|
| 3.3V (Pin 1) | 3.3V |
| GND (Pin 6) | GND |
| GPIO 2 / SDA (Pin 3) | SDA |
| GPIO 3 / SCL (Pin 5) | SCL |

The breakout is a **3.3V** board without level shifting — do not feed it 5V.

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key (address is decimal: 68 = 0x44)
```

Enable I2C (`sudo raspi-config` > Interface Options > I2C).

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
hardware — it then serves a plausible indoor-light scene shifting between
warm and cool white (works with both transports). Useful for testing the app
against a node on any machine.

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

Returns e.g.:

```json
{"sensor":"ISL29125","host":"raspberrypi","lux":428,"red":182,"green":255,"blue":97}
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-isl29125.service` (mirror the other
nodes' unit files), then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-isl29125
sudo systemctl start sensor-playground-isl29125
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
