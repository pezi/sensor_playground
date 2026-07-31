# CozIR Sensor Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a CozIR-A CO₂ sensor (temperature, humidity,
CO₂). Over Wi-Fi the Sensor Tester app discovers this node via UDP
broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header); over BLE the node advertises the Sensor Tester GATT
service instead.

Unlike the other environment sensors the CozIR is **not an I2C device**:
it talks a simple ASCII command protocol over a **9600-baud UART**
(serial), as in the
[dart_periphery serial_cozir.dart example](https://github.com/pezi/dart_periphery/blob/main/example/serial_cozir.dart):

| Command | Effect |
|---------|--------|
| `M 4164` | Select humidity, temperature and CO₂ output fields |
| `K 2` | Polling mode |
| `Q` | One measurement: `H 00495 T 01234 Z 06399` → 49.5 %RH, 23.4 °C, 639.9 ppm |

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_cozir/` and supports the same two transports (Wi-Fi
and BLE).

## Supported Platforms

| Board | Serial port (`serial_port` in config) |
|-------|---------------------------------------|
| Raspberry Pi | `/dev/serial0` (default) |
| NanoPi / Banana Pi (Armbian) | e.g. `/dev/ttyS1` |

### Wiring (UART)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 14 / TXD (Pin 8) | Rx |
| GPIO 15 / RXD (Pin 10) | Tx |

The CozIR is a 3.3 V device — do **not** connect it to 5 V.

Enable the serial port on the Pi (and disable the login shell on it):

```bash
sudo raspi-config   # Interface Options > Serial Port:
                    #   login shell over serial -> No
                    #   serial port hardware    -> Yes
```

## Software Setup

```bash
# Create virtual environment
python3 -m venv venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Copy and edit configuration
cp config.example.json config.json
# Edit config.json: set api_key and the serial_port for your board
```

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

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-cozir.service`:

```ini
[Unit]
Description=Sensor Tester CozIR Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-cozir
ExecStart=/home/pi/sensor-cozir/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-cozir
sudo systemctl start sensor-tester-cozir
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
