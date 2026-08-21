# Grove 125KHz RFID Reader Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a Grove 125KHz RFID Reader (EM4100-family tags) — a *push* node:
instead of serving readings over REST it pushes one JSON message per
scanned tag over a **WebSocket** (`ws://`, port 9132, `X-Api-Key`
checked on the handshake), plus UDP discovery on port 9133.

It is the Node.js counterpart of [`../../python/rfid/`](../../python/rfid/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to the WebSocket with a warning (use the Python or Rust node for BLE).

> ⚠️ **Untested on real hardware.** Like the Python node, this port has
> so far only been tested in [emulation mode](#emulation). The Grove
> reader needs **5V** for reliable operation, but the Raspberry Pi's
> GPIOs are **3.3V-only and not 5V-tolerant**, so the reader's TX line
> must not be connected to the Pi directly — see
> [Wiring](#wiring-uart). The frame parsing itself is identical to the
> tested ESP32 implementation (`../../esp32/esp32_rfid/`).

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
(`0x0024ADAB` = 2403755).

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

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, serial_port
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
reader hardware — it then pushes a fake tag from a small pool every few
seconds. Works on any machine (macOS/Windows included).

## Usage

```bash
node sensor_node.js
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Hold a tag near the antenna; one `{"tag": ...}` message appears per
scan.

`npm test` checks the RDM630 frame parser and XOR checksum against
frames fed byte-by-byte, including resync, overflow and noise cases.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-rfid-node.service`.

## Deviations from the Python node

- BLE transport is not supported; `"transport": "ble"` falls back to
  the WebSocket with a warning.
- The serial port is configured with `stty(1)` and read as a plain file
  (like the CozIR node), so real-hardware mode requires Linux;
  emulation works anywhere.
- Frames are parsed from the read stream's data events instead of the
  Python node's 50 ms polls; the wire behavior is identical.
