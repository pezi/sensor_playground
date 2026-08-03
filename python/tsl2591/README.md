# TSL2591 Sensor Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a TSL2591 I2C light sensor. It reports **visible
light, infrared and illuminance**. Over Wi-Fi the Sensor Tester app discovers
this node via UDP broadcast (port 9133) and polls it for data over HTTPS
(port 9132, `X-Api-Key` header); over BLE the node advertises the Sensor Tester
GATT service instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_tsl2591/` and supports the same two transports (Wi-Fi and
BLE).

The sensor covers roughly **188 µlx to 88 klx** — moonlight to direct sun.
No single gain setting spans that, so the node shifts gain to follow the light
(see below).

## Readings

```json
{"sensor": "TSL2591", "host": "raspberrypi",
 "visible": 4231, "ir": 1187, "lux": 312.4}
```

| Key | Unit | Meaning |
|-----|------|---------|
| `visible` | counts | Broadband channel minus the infrared channel. |
| `ir` | counts | Infrared channel (channel 1). |
| `lux` | lx | Calibrated illuminance, computed from both channels. |

The UDP discovery reply carries the same values under the short keys `vis`,
`ir` and `lux`, so the app's scan list previews them.

## Visible light, infrared and lux

The chip has two photodiodes. Channel 0 is **broadband** — it sees visible
light *and* infrared. Channel 1 is **infrared only**. Neither is "visible
light" on its own; the difference of the two is:

```
visible = broadband - ir
```

The node floors that at zero, because in near-darkness noise can put the IR
channel marginally above the broadband one, and a negative amount of visible
light would be nonsense on a chart.

> The driver's own `visible` property subtracts the IR channel from the
> *packed 32-bit* luminosity word rather than from channel 0, which is not the
> same quantity. This node therefore computes it from `raw_luminosity` the way
> the ESP32 sketch does, so both nodes report identical numbers.

`lux` is a different thing from either count. The driver applies the
datasheet's coefficients to both channels and divides out the current gain and
integration time. Both counts are reported alongside it so that a saturated or
IR-heavy scene is visible rather than hidden behind a single number.

**A saturated reading has no lux.** When a channel pins at 0xFFFF the driver
raises, and the node then omits the `lux` key and sends only the counts,
rather than publishing a number it knows to be wrong. The app's metric registry
renders whatever keys are present, so the card simply disappears for that
reading.

## Gain and integration time

`25x` gain with `300 ms` integration suits normal indoor light. Outdoors in
daylight use `"gain": "low"` with `"integration_ms": 100`; for moonlight use
`"gain": "max"`.

With `auto_gain` the node moves **one step at a time**: up when the broadband
channel falls to 100 counts, down when it saturates. One step, because changing
the gain invalidates the integration already in flight — the *next* reading is
the one that benefits — and because jumping straight to the extreme makes the
value oscillate whenever the light sits near a threshold. Set it to `false`
when you want readings taken at one fixed sensitivity, e.g. to compare counts
across a session.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

### Wiring (I2C)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VIN |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

The I2C address is **fixed at 0x29** — there is no address pin. That is the
same address the **TCS34725** uses, so those two sensors cannot share a bus.

Enable I2C on the Pi:

```bash
sudo raspi-config   # Interface Options > I2C > Enable
```

Check the sensor is seen (it should show at `29`):

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
hardware — it then drifts a lit room's light level on a slow sine around a few
hundred lux, holding the infrared channel at the roughly one-third share of the
broadband count that daylight and incandescent lamps produce, so the three
values stay consistent (works with both transports).

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `i2c_bus` | `1` | I2C bus the sensor is on. |
| `gain` | `"med"` | `low` (1x), `med` (25x), `high` (428x) or `max` (9876x). |
| `integration_ms` | `300` | 100, 200, 300, 400, 500 or 600. Longer collects more light and resolves finer, but each reading takes that long. |
| `auto_gain` | `true` | Follow the light level by shifting gain — see above. |

An unknown `gain` or `integration_ms` fails at startup with the list of valid
values rather than silently falling back to a default, so a typo in the config
cannot quietly change what the readings mean.

## Setup

```bash
python3 -m venv --system-site-packages venv
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

```bash
curl -k -H 'X-Api-Key: your-sensor-api-key' https://localhost:9132/
```

Sanity checks: a lit living room reads a few hundred lux, an office around
500 lx, and covering the sensor with a hand should drop it to single digits.
Point a TV remote at it and press a button — `ir` jumps while `visible` barely
moves.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-tsl2591.service`:

```ini
[Unit]
Description=Sensor Tester TSL2591 Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-tsl2591
ExecStart=/home/pi/sensor-tsl2591/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-tsl2591
sudo systemctl start sensor-tester-tsl2591
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
