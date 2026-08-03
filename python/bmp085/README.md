# BMP085 Sensor Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a BMP085 I2C barometer — the sensor behind the
Grove Barometer Sensor, and the ancestor of the BME280 (no humidity, no gas
sensor). It reports **temperature, barometric pressure and altitude**.
Over Wi-Fi the Sensor Tester app discovers this node via UDP broadcast
(port 9133) and polls it for data over HTTPS (port 9132, `X-Api-Key`
header); over BLE the node advertises the Sensor Tester GATT service
instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_bmp085/` and supports the same two transports (Wi-Fi
and BLE).

The pin-compatible **BMP180** uses the same registers and works with this
node unchanged.

## Readings

```json
{"sensor": "BMP085", "host": "raspberrypi",
 "temperature": 21.3, "pressure": 1013.25, "altitude": -0.1}
```

| Key | Unit | Meaning |
|-----|------|---------|
| `temperature` | °C | Die temperature. |
| `pressure` | hPa | Compensated barometric pressure. |
| `altitude` | m | Derived from the pressure — see below. |

The UDP discovery reply carries the same values under the short keys `temp`,
`press` and `alt`, so the app's scan list previews them.

## About the altitude

It is computed from the pressure against the standard sea-level value of
**1013.25 hPa**, using the international barometric formula the datasheet
quotes. That makes it an absolute altitude only when the weather happens to
match the standard atmosphere — on a low-pressure day a stationary sensor
reads a few dozen metres too high. As a *relative* measurement (lift the
sensor, watch it climb) it is precise to well under a metre.

## Why there is no driver package

The BMP085 returns **uncompensated** readings: two raw ADC values that only
become a temperature and a pressure after Bosch's fixed-point compensation,
using eleven calibration constants read from the chip's EEPROM. That
arithmetic is in `compensate()` here and mirrored in
`../../esp32/esp32_bmp085/esp32_bmp085.ino`, so both nodes produce identical
numbers from identical raw readings.

It is verified against the worked example published in the datasheet
(section 3.5): calibration `AC1=408 … MD=2868` with `UT=27898`, `UP=23843`
must yield exactly **15.0 °C** and **69964 Pa**.

> One subtlety worth keeping: the datasheet writes one step as a division
> rather than a shift, and its numerator is negative. C truncates toward zero
> there while Python's `//` floors, which would shift the result by one step
> (0.1 °C, plus a little pressure). `_trunc_div()` exists to match C.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

### Wiring (I2C)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

The I2C address is **fixed at 0x77** — the BMP085 has no address pin, so only
one can sit on a bus.

Enable I2C on the Pi:

```bash
sudo raspi-config   # Interface Options > I2C > Enable
```

Check the sensor is seen (it should show at `77`):

```bash
i2cdetect -y 1
```

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — HTTPS REST on port 9132 plus UDP discovery on
  port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketch
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the sensor
hardware — it then drifts temperature and pressure on slow sines around
21 °C / 1013 hPa and derives the altitude from its own emulated pressure, so
the three values stay consistent (works with both transports).

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `i2c_bus` | `1` | I2C bus the sensor is on. |
| `oversampling` | `3` | 0–3. Higher means less noise and a longer conversion (5 ms at 0, 26 ms at 3). 3 is the datasheet's "ultra high resolution" mode. |

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key, i2c_bus
```

For the Wi-Fi transport, generate the self-signed certificate the REST
endpoint serves:

```bash
openssl req -x509 -newkey rsa:2048 -nodes -keyout key.pem -out cert.pem -days 3650 -subj "/CN=sensor"
```

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

```bash
curl -k -H 'X-Api-Key: your-sensor-api-key' https://localhost:9132/
```

Sanity checks: the pressure should read within a few hPa of your local
airport's QNH, and the altitude should climb by roughly 8 m for every 1 hPa
the pressure drops.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-bmp085.service`:

```ini
[Unit]
Description=Sensor Tester BMP085 Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-bmp085
ExecStart=/home/pi/sensor-bmp085/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-bmp085
sudo systemctl start sensor-tester-bmp085
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
