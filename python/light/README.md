# Grove Light Sensor Node for Sensor Tester (Python)

This Python script reads a **Grove Light Sensor** (an analog photo-resistor)
and serves a raw brightness value to the Sensor Tester app. Over Wi-Fi the
Sensor Tester app discovers this node via UDP broadcast (port 9133) and polls
it for data over HTTPS (port 9132, `X-Api-Key` header); over BLE the node
advertises the Sensor Tester GATT service instead.
https://wiki.seeedstudio.com/Grove-Light_Sensor/

> **Pollable, not push.** The light sensor produces a continuous value, so it
> uses the same pollable transports as the environment sensors. The app shows
> it as a normal "Brightness" live-data card and chart.

It is the Python/SoC counterpart of `../../esp32/esp32_light/` and supports
the same two transports (Wi-Fi and BLE).

## Needs an ADC hat

The Raspberry Pi has **no analog input**, so an analog sensor must go through
an extension hat that has an ADC. This node reads it via the sibling
[`extension_hat`](../extension_hat/) helper — there is no direct-GPIO option:

| `hat_type` | Hardware | Resolution |
|------------|----------|------------|
| `"grove"` | Seeed Grove Base Hat (`read_adc_raw`) | 12-bit, 0-4095 |
| `"nano"` | FriendlyARM NanoHat Hub (`analog_read`) | 10-bit |
| `"grovePlus"` | Seeed GrovePi+ (`analog_read`) | 10-bit |

`pin` is the hat's analog channel (`0`-`7` on the Grove Base Hat). Higher
values mean brighter; the reading is **not** calibrated to lux.

### Wiring

Plug the Grove Light Sensor into an **analog** port of the hat (e.g. `A0` on
the Grove Base Hat → `pin: 0`).

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key, hat_type, pin
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

Returns e.g.:

```json
{"sensor":"LIGHT","host":"raspberrypi","light":2731}
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-light.service` (mirror the other
nodes' unit files), then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-light
sudo systemctl start sensor-tester-light
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
