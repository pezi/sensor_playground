# PAJ7620 Sensor Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a PAJ7620 I2C sensor (hand gestures).
It is a *push* node: instead of serving readings over REST it pushes
one JSON message per event over a **WebSocket** (`ws://`, port 9132,
`X-Api-Key` checked on the handshake). The app discovers it via UDP
broadcast on port 9133. Over BLE the node advertises the Sensor Tester
GATT service instead and sends each event as a notification.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_paj7620/` and supports the same two transports (Wi-Fi
and BLE).

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket push server on port 9132 plus UDP
  discovery on port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery. Only
  one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the sensor
hardware — it then pushes plausible generated readings (works with both
transports). Useful for testing the app against a node on any machine.

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

```bash
# Create virtual environment
python3 -m venv venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Copy and edit configuration
cp config.example.json config.json
# Edit config.json: set api_key and the i2c_bus for your board
```

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://localhost:9132/
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-paj7620.service`:

```ini
[Unit]
Description=Sensor Tester PAJ7620 Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-paj7620
ExecStart=/home/pi/sensor-paj7620/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-paj7620
sudo systemctl start sensor-tester-paj7620
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.

## Notes

The driver comes from Seeed's [grove.py](https://github.com/Seeed-Studio/grove.py).
If `pip install grove.py` fails on your board, use Seeed's installer:

```bash
curl -sL https://github.com/Seeed-Studio/grove.py/raw/master/install.sh | sudo bash -s -
```
