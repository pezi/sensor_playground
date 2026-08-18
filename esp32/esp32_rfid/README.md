# ESP32 Grove 125KHz RFID Reader Node — Sensor Tester

ESP32 sketch for the [Grove 125KHz RFID
Reader](https://wiki.seeedstudio.com/Grove-125KHz_RFID_Reader/), an
EM4100-tag reader on a 9600-baud UART. It is a *push* node: an RFID
reader only produces data at the instant a tag is scanned, so instead of
serving readings over REST the node pushes one JSON message per scan.

> **The jumper on the reader must be in UART mode** (not Wiegand) —
> in Wiegand mode nothing ever arrives on the serial line.

## Transport

Selected at compile time via `ACTIVE_TRANSPORT` in the sketch (or the
`install.sh` argument):

| Transport | Wire protocol |
|-----------|---------------|
| `TRANSPORT_WIFI` | WebSocket server on port 9132 (`ws://`, `X-Api-Key` handshake header) + UDP discovery on port 9133 |
| `TRANSPORT_BLE` (default) | Sensor Tester GATT service; each scan is one notify on the data characteristic |

## Protocol (Wi-Fi)

- UDP discovery: the node answers a `SENSOR_TESTER` broadcast on port
  9133 with `{"type":"RFID","host":"...","ip":"...","port":9132}`.
- WebSocket push, one message per scan:

```json
{"tag": "0F0024ADAB"}
```

The value is the 10 ASCII-hex data characters of the RDM630-style frame
(`STX 0x02 | 10 hex data chars | 2 hex checksum chars | ETX 0x03`). The
node verifies the XOR-of-five-data-bytes checksum and drops corrupt
frames. Repeats of the same tag are suppressed for 2 seconds while it
is held near the antenna. The decimal number printed on most tags is
the low 32 bits (`0x0024ADAB` = 2403755).

## Protocol (BLE)

| Characteristic | UUID | Purpose |
|----------------|------|---------|
| Service | `d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f` | advertised as `RFID` |
| Data | `d1a51b00-0002-...` | READ + NOTIFY — one `{"tag":...}` per scan (chunked 0x1E framing, see `../common/sensor_ble_framing.h`) |
| Auth | `d1a51b00-0003-...` | WRITE — the shared API key |

## Hardware Requirements

- ESP32 dev board
- Grove 125KHz RFID Reader (jumper on UART) + an EM4100/EM4102 tag

### Wiring (UART)

| ESP32 Pin | Reader Pin |
|-----------|------------|
| 5V | VCC |
| GND | GND |
| GPIO 16 (RX2) | TX |

The reader is transmit-only; its RX pin stays unconnected. There is
nothing to probe at boot — a wiring error shows only as silence.

## Software Requirements

1. Arduino IDE or `arduino-cli`
2. ESP32 board support (`esp32:esp32`)
3. Libraries: `ArduinoJson`, and for Wi-Fi `WebSockets`
   (Markus Sattler / Links2004)

### Quick install (arduino-cli)

```bash
./install.sh /dev/cu.usbserial-0001 WIFI   # or BLE (default)
```

## Configuration

### Secrets

On the first run `install.sh` creates `secrets.h` from
`secrets.h.example`; fill in the Wi-Fi credentials, the API key (min. 8
characters, must match the key configured in the Sensor Tester app) and
the hostname, then re-run.

## Testing

Wi-Fi:

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Hold a tag near the antenna; one `{"tag": ...}` message appears per
scan. Over BLE, use nRF Connect: write the API key to the auth
characteristic, subscribe to the data characteristic, and scan a tag.

## See also

- `../../python/rfid/` — the same node for Raspberry Pi & co.
