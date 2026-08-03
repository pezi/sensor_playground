# Rotary Angle Sensor Node for Sensor Tester (Python)

Reads a [Grove Rotary Angle Sensor](https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/)
— a 10 kΩ potentiometer with 300° of mechanical travel — through an extension
hat's ADC and reports the knob position to the Sensor Tester app, which draws
it as a needle on a dial.

Unlike the light sensor (also analog, but polled over HTTPS every few seconds)
this is a **push** node: the app's needle tracks the knob, so a reading that is
seconds old is useless. The node samples continuously and sends a message
whenever the knob has moved past the deadband — over Wi-Fi on a **WebSocket**
(`ws://`, port 9132, `X-Api-Key` checked on the handshake), discovered via UDP
broadcast on port 9133. Over BLE the node advertises the Sensor Tester GATT
service instead and sends each movement as a notification.

It is the Python/SoC counterpart of `../../esp32/esp32_rotary_angle/` and
supports the same two transports.

## Protocol

```json
{"adc": 2048, "adcMax": 4095, "angle": 150.1, "angleMax": 300.0}
```

| Key | Meaning |
|-----|---------|
| `adc` | Raw ADC count, `0 … adcMax`. |
| `adcMax` | Full-scale count of **this** hat — see below. |
| `angle` | Shaft position in degrees, `adc / adcMax × angleMax`. |
| `angleMax` | Mechanical travel of the knob (300° for the Grove sensor). |

The node sends the current position once right after a client connects, then
one message per movement.

## Supported hats and their ADC range

The Raspberry Pi has no analog input, so an analog sensor always needs a hat
with an ADC — there is no direct-GPIO option (unlike the digital contact node).
The converter's width belongs to the hat, not to the knob, and the supported
hats do not agree:

| `hat_type` | Hat | Resolution | Range |
|------------|-----|-----------|-------|
| `"grove"` | Seeed Grove Base Hat | 12-bit | `0 – 4095` |
| `"nano"` | FriendlyARM NanoHat Hub (BakeBit) | 10-bit | `0 – 1023` |
| `"grovePlus"` | Seeed GrovePi+ | 10-bit | `0 – 1023` |
| — | *(the ESP32 sketch)* | 12-bit | `0 – 4095` |

A bare `512` is therefore ambiguous — half travel on a NanoHat, an eighth on a
Grove Base Hat. That is why `adcMax` travels with **every** message: the app
scales its dial to whichever node it is talking to instead of assuming one of
them. `angle`/`angleMax` carry the same position in degrees, so the app never
needs to know the knob's mechanical travel either.

The node picks the right `adcMax` from `hat_type` automatically — there is
nothing to configure for it.

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket push server on port 9132 plus UDP
  discovery on port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketch
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery. Only
  one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the sensor
hardware — it then sweeps slowly from end to end and back, as if someone were
turning the knob (works with both transports). The emulated node reports the
range the configured `hat_type` really would, so the app's dial can be checked
against a 10-bit node as easily as a 12-bit one.

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `hat_type` | `"grove"` | Which hat reads the ADC; also fixes `adcMax`. |
| `pin` | `0` | ADC channel / port the sensor is plugged into. |
| `i2c_bus` | `1` | I2C bus of the hat (`0` on a NanoHat Hub). |
| `angle_max` | `300` | Mechanical travel of the knob, in degrees. |
| `deadband` | *(auto)* | How far the count must move before a message goes out. Defaults to ~0.6 % of full scale — 24 counts on a 12-bit hat, 6 on a 10-bit one. Raise it if a resting knob still chatters. |

### Wiring

| Hat | Connect to |
|-----|-----------|
| Grove Base Hat | any `A0`–`A7` port; `pin` is the channel number |
| NanoHat Hub / GrovePi+ | any `A0`–`A2` port; `pin` is the port number |

The hats supply the sensor at their own reference voltage, so unlike the ESP32
wiring there is no 3.3 V/5 V pitfall here.

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt

cp config.example.json config.json
# Edit: api_key, hat_type, pin (and i2c_bus for a NanoHat Hub)
```

Enable I2C first (`sudo raspi-config` > Interface Options > I2C).

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

The node prints its ADC range on startup, then a line per movement.

## Testing

With [`websocat`](https://github.com/vi/websocat):

```bash
websocat -H='X-Api-Key: your-sensor-api-key' ws://localhost:9132/
```

The current position prints on connect; turning the knob prints a JSON line per
movement. Turn it to both end stops and check that `adc` reaches `0` and
`adcMax` exactly.

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-tester-rotary.service`:

```ini
[Unit]
Description=Sensor Tester Rotary Angle Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-rotary_angle
ExecStart=/home/pi/sensor-rotary_angle/venv/bin/python3 sensor_node.py
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-tester-rotary
sudo systemctl start sensor-tester-rotary
```

For the BLE transport, depend on Bluetooth instead of the network and make
sure the service user may register GATT applications (`bluetooth` group or
`User=root`): replace the `[Unit]` dependencies with `After=bluetooth.target`
/ `Wants=bluetooth.target` and set `User=root`.
