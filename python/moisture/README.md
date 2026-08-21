# Grove Capacitive Moisture Sensor Node for Sensor Playground (Python)

This Python script reads a **Grove Capacitive Moisture Sensor
(Corrosion-Resistant)** and serves the soil moisture as a percentage to the
Sensor Playground app. Over Wi-Fi the Sensor Playground app discovers this node via
UDP broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header); over BLE the node advertises the Sensor Playground GATT
service instead.
https://wiki.seeedstudio.com/Grove-Capacitive_Moisture_Sensor-Corrosion-Resistant/

The probe is capacitive: no exposed metal electrodes, so unlike a resistive
probe it can stay in damp soil permanently without corroding. Its analog
output voltage **falls** as the soil gets wetter.

> **Pollable, not push.** The probe produces a continuous value, so it uses
> the same pollable transports as the environment sensors. The app shows it
> as a normal "Soil Moisture" live-data card and chart.

It is the Python/SoC counterpart of `../../esp32/esp32_moisture/` and supports
the same two transports (Wi-Fi and BLE).

## Needs an ADC hat

The Raspberry Pi has **no analog input**, so an analog sensor must go through
an extension hat that has an ADC. This node reads it via the sibling
[`extension_hat`](../extension_hat/) helper — there is no direct-GPIO option:

| `hat_type` | Hardware | Resolution |
|------------|----------|------------|
| `"grove"` | Seeed Grove Base Hat (`read_adc_raw`) | 12-bit, 0-4095 |
| `"nano"` | FriendlyARM NanoHat Hub (`analog_read`) | 10-bit, 0-1023 |
| `"grovePlus"` | Seeed GrovePi+ (`analog_read`) | 10-bit, 0-1023 |

`pin` is the hat's analog channel (`0`-`7` on the Grove Base Hat).

### Wiring

Plug the probe into an **analog** port of the hat (e.g. `A0` on the Grove
Base Hat → `pin: 0`). Do not insert the probe deeper than the marked line —
the electronics at the top are not waterproof.

## Calibration

`moisture` is a `0`-`100` % value mapped linearly between two calibration
points in `config.json`:

| Key | Meaning | Default |
|-----|---------|---------|
| `adc_dry` | averaged raw ADC reading in dry air | `2600` |
| `adc_wet` | averaged raw ADC reading submerged in water | `1100` |

The defaults are 12-bit Grove Base Hat starting points; on a **10-bit** hat
divide them by four (e.g. `650` / `275`) before fine-tuning. Probes and hats
vary, so **calibrate yours**: the REST payload also carries the averaged raw
reading (`adc`, with its full scale `adcMax`). Hold the probe in dry air,
read `adc`, put the number into `adc_dry`; submerge it up to the marked line
in a glass of water and do the same for `adc_wet`. Values outside the
calibrated span clamp to 0/100 %.

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key, hat_type, pin, adc_dry, adc_wet
```

Enable I2C (`sudo raspi-config` > Interface Options > I2C) — the ADC hat is an
I2C device at address 0x04.

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
hardware — it then serves a plausible watering-and-drying cycle (works with
both transports). Useful for testing the app against a node on any machine.

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
{"sensor":"MOISTURE","host":"raspberrypi","moisture":63,"adc":1820,"adcMax":4095}
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-moisture.service` (mirror the other
nodes' unit files), then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-moisture
sudo systemctl start sensor-playground-moisture
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
