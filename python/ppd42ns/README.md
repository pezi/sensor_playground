# Grove Dust Sensor (PPD42NS) Node for Sensor Tester (Python)

This Python script reads a **Grove Dust Sensor** (Shinyei PPD42NS) and serves
a particle concentration to the Sensor Tester app. Over Wi-Fi the Sensor
Tester app discovers this node via UDP broadcast (port 9133) and polls it for
data over HTTPS (port 9132, `X-Api-Key` header); over BLE the node advertises
the Sensor Tester GATT service instead.
https://wiki.seeedstudio.com/Grove-Dust_Sensor/

> **Pollable, not push.** The dust value is continuous, so the node uses the
> same pollable transports as the environment sensors. The app shows it as a
> classified air-quality card (Dylos bands) plus a live chart.

It is the Python/SoC counterpart of `../../esp32/esp32_ppd42ns/` and supports
the same two transports (Wi-Fi and BLE).

## How the value is measured

The PPD42NS pulls its output pin LOW while particles scatter light inside its
chamber (pulses of roughly 10–90 ms). The node timestamps both edges with
gpiozero callbacks and accumulates the **low-pulse occupancy (LPO)** over
30-second windows:

    ratio         = low_time / window_time * 100          (percent)
    concentration = 1.1·r³ − 3.8·r² + 520·r + 0.62        (pcs/0.01cf)

(the Nafis curve, https://www.howmuchsnow.com/arduino/airquality/grovedust/).

- **The first reading appears after the first full 30-second window.** Until
  then the REST endpoint answers 503 ("Sensor read failed") — that is warm-up,
  not a fault.
- Shinyei recommends letting the sensor stabilize for ~3 minutes after
  power-on before trusting the values.
- gpiozero delivers edge callbacks with about a millisecond of jitter; both
  edges pass through the same path, so the error largely cancels and averages
  out over a 30-second window. If readings look noisy on a heavily loaded
  board, switch gpiozero to the `pigpio` pin factory, whose daemon timestamps
  edges in hardware.

## Direct GPIO only

The pin is read directly via gpiozero — there is **no extension-hat option**:
the Arduino-based hats are polled over I2C and cannot timestamp 10–90 ms
pulses. `pin` in `config.json` is the BCM GPIO number.

### Wiring

| PPD42NS pin | Connect to |
|-------------|------------|
| 1 (GND, black) | GND |
| 3 (5V, red) | **5 V** (the heater draws ~90 mA — do not use 3V3) |
| 4 (P1 output, yellow) | voltage divider → GPIO (default GPIO 17) |

The output swings up to ~4.5 V, which is **not 3.3 V-safe**: pass it through a
voltage divider (e.g. 10 kΩ from the sensor output to the GPIO, 20 kΩ from
the GPIO to GND ⇒ 4.5 V → 3.0 V). Mount the sensor vertically — its chamber
relies on the heater's convection air flow.

## Setup

```bash
python3 -m venv venv --system-site-packages
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key, pin
```

The venv must be created with `--system-site-packages` so gpiozero can see the
system pin-factory backend (`python3-lgpio` on current Raspberry Pi OS); see
the note in `requirements.txt`.

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
{"sensor":"PPD42NS","host":"raspberrypi","dust":412.5}
```

During the first 30 seconds after start it returns 503 instead — the first
LPO window has not completed yet.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-ppd42ns.service` (mirror the other
nodes' unit files), then:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-ppd42ns
sudo systemctl start sensor-tester-ppd42ns
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
