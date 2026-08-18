# Grove 125KHz RFID Reader Node for Sensor Tester (Python)

This Python script implements the Sensor Tester sensor interface on
single-board computers with a Grove 125KHz RFID Reader (EM4100-family
tags). It is a *push* node: instead of serving readings over REST it
pushes one JSON message per scanned tag over a **WebSocket** (`ws://`,
port 9132, `X-Api-Key` checked on the handshake). The app discovers it
via UDP broadcast on port 9133. Over BLE the node advertises the Sensor
Tester GATT service instead and sends each scan as a notification.

It is the Python/SoC counterpart of the ESP32 sketch in
`../../esp32/esp32_rfid/` and supports the same two transports (Wi-Fi
and BLE).

> ⚠️ **Untested on real hardware.** Unlike the ESP32 sketch (verified
> with a reader and tags), this Python node has so far only been tested
> in [emulation mode](#emulation). The Grove reader needs **5V** for
> reliable operation, but the Raspberry Pi's GPIOs are **3.3V-only and
> not 5V-tolerant**, so the reader's TX line must not be connected to
> the Pi directly — see [Wiring](#wiring-uart) for the two ways to
> bridge 5V → 3.3V. The frame parsing itself is identical to the tested
> ESP32 implementation.

## Protocol

One message per scan:

```json
{"tag": "0F0024ADAB"}
```

The value is the 10 ASCII-hex data characters of the RDM630 frame the
reader emits (2 version/customer chars + 8 chars of 32-bit tag id). The
node verifies the frame's XOR checksum before publishing and suppresses
repeats of the same tag for 2 seconds while it is held near the
antenna. The decimal number printed on most tags is the low 32 bits
(`int("0024ADAB", 16)` = 2403755).

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

Set `"emulation": true` in `config.json` to run the node without the
reader hardware — it then pushes a fake tag from a small pool every few
seconds (works with both transports). Useful for testing the app
against a node on any machine.

## Hardware

- [Grove 125KHz RFID Reader](https://wiki.seeedstudio.com/Grove-125KHz_RFID_Reader/)
- **The jumper on the reader must be in UART mode** (not Wiegand);
  9600 baud, 8N1. Only the reader's TX line is used.

### Wiring (UART)

| Pi Pin | Reader Pin |
|--------|------------|
| 5V (Pin 2) | VCC |
| GND (Pin 6) | GND |
| GPIO 15 / RXD (Pin 10) | **TX — via level shifter or voltage divider, see below!** |

The reader needs **5V** on VCC for reliable operation, so its TX line
also swings at 5V TTL. The Pi's GPIOs are **3.3V-only and not
5V-tolerant** — connecting the reader's TX to the Pi's RXD directly can
damage the Pi. Since the reader is transmit-only, only this one line
(5V → 3.3V) has to be shifted; there are two easy ways:

1. **Level shifter module** — one channel of a ready-made bidirectional
   logic level converter (e.g. a BSS138-based 4-channel board): reader
   TX → HV input, LV output → Pi RXD; connect HV to 5V, LV to 3.3V and
   the grounds together.

2. **Resistor voltage divider** — two resistors on the TX line divide
   5V down to ~3.3V; entirely sufficient for a slow 9600-baud line:

   ```text
   Reader TX ──[ 1 kΩ ]──┬──────────► Pi RXD (Pin 10)
                         │
                      [ 2 kΩ ]
                         │
                        GND
   ```

   (5V × 2k/(1k+2k) ≈ 3.3V. Any pair with the same ~1:2 ratio works,
   e.g. 1.8 kΩ / 3.3 kΩ.)

On a Raspberry Pi the primary UART is `/dev/serial0`. Disable the
serial login console first (`sudo raspi-config` → Interface Options →
Serial Port → login shell **No**, serial hardware **Yes**) and reboot.
Other boards: set `"serial_port"` in `config.json` to the UART device
the reader is wired to (e.g. `/dev/ttyS1`).

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `api_key` | — | Shared key the app must present (min. 8 characters) |
| `hostname` | `""` | Name shown in the app; empty = system hostname |
| `serial_port` | `/dev/serial0` | UART device the reader is wired to |
| `transport` | `wifi` | `wifi` (WebSocket + UDP discovery) or `ble` |
| `emulation` | `false` | Generate fake scans without hardware |

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
cp config.example.json config.json   # then edit it
```

## Usage

```bash
python3 sensor_node.py
```

## Testing

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Hold a tag near the antenna; one `{"tag": ...}` message appears per
scan.

## Running as a Service (optional)

```ini
# /etc/systemd/system/sensor-tester-rfid.service
[Unit]
Description=Sensor Tester RFID node
After=network-online.target

[Service]
WorkingDirectory=/home/pi/rfid
ExecStart=/home/pi/rfid/venv/bin/python3 sensor_node.py
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

For the BLE transport add `After=bluetooth.target` and run as
`User=root`.
