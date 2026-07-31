# IMU 10DOF Sensor Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a
[Grove IMU 10DOF](https://wiki.seeedstudio.com/Grove-IMU_10DOF/) board
(MPU9250 + BMP280). The MPU9250 axes are reduced to roll, pitch and compass
heading angles plus the total acceleration magnitude (g-force); the BMP280
adds temperature and barometric pressure. Over Wi-Fi the Sensor Tester app
discovers this node via UDP broadcast (port 9133) and then **streams** the
readings from a WebSocket (port 9132, `ws://`, `X-Api-Key` header on the
handshake) — the app streams motion sensors rather than polling them; over
BLE the node advertises the Sensor Tester GATT service instead.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_imu10dof/` and supports the same two transports (Wi-Fi
and BLE).

> Roll/pitch come from the accelerometer and the heading from the raw
> magnetometer (not tilt-compensated), so the heading is most accurate when
> the board lies flat. The BMP280 is driven directly via smbus2 using the
> Bosch datasheet algorithm — no extra driver package needed.

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

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket push server (`ws://`) on port 9132 plus UDP
  discovery on port 9133. Readings are pushed every 250 ms while a client is
  connected. No SSL certificates are needed (the app connects over `ws://`).
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No SSL certificates and no UDP discovery; BLE advertising
  is the discovery. Only one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the sensor
hardware — it then serves plausible generated readings (works with both
transports). Useful for testing the app against a node on any machine.

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

```bash
# stream readings (websockets package provides the client)
python3 -c "
import asyncio, websockets
async def main():
    async with websockets.connect('ws://localhost:9132',
                                  additional_headers={'X-Api-Key': 'your-sensor-api-key'}) as ws:
        for _ in range(5): print(await ws.recv())
asyncio.run(main())
"
```

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-imu10dof.service`:

```ini
[Unit]
Description=Sensor Tester IMU 10DOF Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-imu10dof
ExecStart=/home/pi/sensor-imu10dof/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-imu10dof
sudo systemctl start sensor-tester-imu10dof
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
