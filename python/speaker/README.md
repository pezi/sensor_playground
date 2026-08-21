# Grove Speaker Node for Sensor Playground (Python)

This Python script implements the Sensor Playground sensor interface on
single-board computers with a
[Grove Speaker](https://wiki.seeedstudio.com/Grove-Speaker/) — a small
amplified loudspeaker on a digital pin. It is an *actuator* node like the
LED: the app plays single notes and a built-in melody on it, and the node
reports what is actually sounding back — a tone ends on its own when its
duration runs out, so the app follows the node's state rather than its
own taps.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_speaker/` and supports the same two transports (Wi-Fi
and BLE).

The square wave comes from lgpio's software PWM at 50% duty; the Grove
module's onboard transistor amplifies it. A Grove Buzzer works too, at
reduced sound quality. There is no extension-hat option: the
Arduino-based hats cannot stream a changing PWM frequency.

## Protocol

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"tone":{"freq":440,"ms":400}}` | play one tone |
| app → node | `{"melody":true}` | play the built-in melody |
| app → node | `{"stop":true}` | silence |
| node → app | `{"freq":440}` / `{"freq":0}` | what is sounding — sent once right after connect and on every change, including a tone ending on its own |

Over BLE the state carries the same JSON as a notify, and a command is a
short binary write: `0x01 <freq:u16 BE> <ms:u16 BE>` (tone), `0x02`
(melody), `0x03` (stop).

## Supported Platforms

| Board | Notes |
|-------|-------|
| Raspberry Pi | `gpio_chip: 0`; the speaker pin is a plain GPIO |

### Wiring

| Pi Pin | Grove Pin |
|--------|-----------|
| 5V (Pin 2) or 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 5 (Pin 29, `speaker_pin` in config) | SIG (yellow) |

> The module is louder at 5V; the SIG line only carries the Pi's output,
> so either supply is safe.

## Software Setup

```bash
# Create virtual environment — --system-site-packages lets it see the
# system python3-lgpio, which cannot be pip-installed
sudo apt install python3-lgpio
python3 -m venv --system-site-packages venv
source venv/bin/activate

# Install dependencies
pip install -r requirements.txt

# Copy and edit configuration
cp config.example.json config.json
# Edit config.json: set api_key and the speaker_pin your module is wired to
```

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `api_key` | — | Shared key the app must present (min. 8 characters) |
| `hostname` | `""` | Name shown in the app; empty = system hostname |
| `speaker_pin` | `5` | BCM GPIO of the SIG line |
| `gpio_chip` | `0` | gpiochip number (`0` on Raspberry Pi) |
| `transport` | `wifi` | `wifi` (WebSocket + UDP discovery) or `ble` |
| `emulation` | `false` | Print tones instead of driving hardware |

## Transports

The transport is selected via `"transport"` in `config.json`:

Both transports import the shared `../common/` folder, so deploy it next
to this node folder.

- `"wifi"` (default) — WebSocket server on port 9132 plus UDP discovery
  on port 9133.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  (see `../common/README.md` for the GATT contract, BlueZ prerequisites
  and testing). No UDP discovery; BLE advertising is the discovery. Only
  one BLE node can run per board.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
speaker hardware — it then prints the sounding state instead (works with
both transports). Useful for testing the app against a node on any
machine.

## Usage

```bash
source venv/bin/activate
python3 sensor_node.py
```

## Testing

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

The current state prints on connect. Type
`{"tone":{"freq":440,"ms":1000}}` and press enter — the speaker sounds an
A for one second and the node echoes `{"freq":440}`, then `{"freq":0}`
when the tone ends.

## Running as a Service (optional)

```ini
# /etc/systemd/system/sensor-playground-speaker.service
[Unit]
Description=Sensor Playground speaker node
After=network-online.target

[Service]
WorkingDirectory=/home/pi/speaker
ExecStart=/home/pi/speaker/venv/bin/python3 sensor_node.py
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

For the BLE transport add `After=bluetooth.target` and run as
`User=root`.
