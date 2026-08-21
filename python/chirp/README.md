# Chirp I2C Soil Moisture Sensor Node for Sensor Playground (Python)

This Python script reads a **Catnip Electronics I2C Soil Moisture Sensor**
(the "Chirp" sensor, default I2C address `0x20`) and serves the soil moisture
as a percentage, the soil temperature and the ambient light level to the
Sensor Playground app. Over Wi-Fi the Sensor Playground app discovers this node via
UDP broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header); over BLE the node advertises the Sensor Playground GATT
service instead.

- Register protocol: https://github.com/Apollon77/I2CSoilMoistureSensor
- Raspberry Pi reference: https://github.com/ageir/chirp-rpi
- Product: https://www.robotshop.com/products/i2c-soil-moisture-sensor

The sensor is capacitive: no exposed metal electrodes, so it can stay in soil
permanently without corroding. Unlike the analog Grove probe it talks plain
I2C — **no ADC hat needed** on the Raspberry Pi.

> **Pollable, not push.** The sensor produces continuous values, so it uses
> the same pollable transports as the environment sensors. The app shows
> "Soil Moisture", "Temperature" and "Brightness" cards/charts.

It is the Python/SoC counterpart of `../../esp32/esp32_chirp/` and supports
the same two transports (Wi-Fi and BLE).

## Readings & calibration

| JSON key (REST) | Meaning |
|-----------------|---------|
| `moisture` | soil moisture, `0`-`100` %, mapped between `cap_dry`/`cap_wet` |
| `temperature` | soil/sensor temperature in °C (chip reports tenths) |
| `light` | ambient brightness in raw counts, **higher = brighter** (the chip counts a phototransistor discharge upward in darkness; the node inverts it, `65535 - raw`) |
| `cap` | the raw capacitance, for calibrating the two points below |

The raw capacitance rises with moisture, so **wet > dry**:

| Key | Meaning | Default |
|-----|---------|---------|
| `cap_dry` | raw capacitance in dry air | `290` |
| `cap_wet` | raw capacitance submerged in water | `520` |

Individual sensors vary, so **calibrate yours**: read `cap` from the REST
payload with the sensor in dry air and again submerged up to the marked line
in a glass of water, and put the two numbers into `config.json`. Values
outside the calibrated span clamp to 0/100 %.

A light measurement takes up to three seconds on the chip, so the value is
harvested from a measurement started on an earlier poll; the `light` key is
absent until the first one completes (a few seconds after start).

### Wiring

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 / SDA (Pin 3) | SDA |
| GPIO 3 / SCL (Pin 5) | SCK/SCL |

Only the coated probe part goes into the soil — keep the components at the
top dry.

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key, cap_dry, cap_wet (address is decimal: 32 = 0x20)
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
hardware — it then serves a plausible watering cycle with temperature and a
day/night light curve (works with both transports). Useful for testing the
app against a node on any machine.

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
{"sensor":"CHIRP","host":"raspberrypi","moisture":47,"temperature":21.3,"light":52000,"cap":400}
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-chirp.service` (mirror the other
nodes' unit files), then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-chirp
sudo systemctl start sensor-playground-chirp
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
