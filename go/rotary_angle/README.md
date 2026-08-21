# Rotary Angle Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a [Grove Rotary Angle Sensor](https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/)
— a 10 kΩ potentiometer with 300° of mechanical travel — read through an
extension hat's ADC. The Sensor Playground app draws the knob position as
a needle on a dial.

Unlike the light sensor (also analog, but polled over HTTPS every few
seconds) this is a **push** node: the app's needle tracks the knob, so a
reading that is seconds old is useless. The node samples continuously and
sends a message whenever the knob has moved past the deadband:

```json
{"adc": 2048, "adcMax": 4095, "angle": 150.1, "angleMax": 300.0}
```

| Key | Meaning |
|-----|---------|
| `adc` | Raw ADC count, `0 … adcMax`. |
| `adcMax` | Full-scale count of the hat (`4095` for the Grove Base Hat). |
| `angle` | Shaft position in degrees, `adc / adcMax × angleMax`. |
| `angleMax` | Mechanical travel of the knob (300° for the Grove sensor). |

It serves a **WebSocket** (`ws://`, port 9132, `X-Api-Key` handshake
header) instead of HTTPS REST, plus UDP discovery on port 9133. On
connect the node first sends the current position.

It is the Go counterpart of
[`../../python/rotary_angle/`](../../python/rotary_angle/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Needs a Grove Base Hat

The Raspberry Pi has no analog input, so the sensor is read through the
Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04): plug the sensor
into one of the hat's analog ports (A0-A7) and set `pin` to that channel.
Enable I2C: `sudo raspi-config` → Interface Options → I2C.

> The Arduino-based hats the Python node also supports (`"nano"`,
> `"grovePlus"`) are **not** implemented in this port — use the Python
> node for those.

`adcMax` travels with every message because the converter's width belongs
to the hat doing the reading, not to the knob: the app scales its dial to
whichever node it is talking to (`4095` here, `1023` on the 10-bit
Arduino-based hats) instead of assuming one of them.

## Deadband

`deadband` in `config.json` sets how far the count must move before a
message goes out. It defaults to ~0.6 % of full scale (25 counts on the
12-bit hat) — raise it if a resting knob still chatters. The ends of
travel are always pinned, so a fully turned knob reports exactly `0` or
`adcMax` instead of stopping a deadband short of it.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_rotary_angle .
cp config.example.json config.json    # edit: api_key, pin
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_rotary_angle .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then sweeps slowly from end to end and back, as if someone
were turning the knob. Works on any machine (macOS/Windows included).

## Usage

```bash
./sensor_node_rotary_angle
```

The node prints its ADC range on startup, then a line per movement.

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# position pushes, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

The current position prints on connect; turning the knob prints a JSON
line per movement. Turn it to both end stops and check that `adc` reaches
`0` and `adcMax` exactly.

The deadband/end-pinning logic is covered by a unit test:
`go test ./rotary_angle/` (from the `go/` folder).

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-rotary_angle-go.service`.

## Deviations from the Python node

- Only the Grove Base Hat (`"hat_type": "grove"`) is implemented;
  `"nano"` / `"grovePlus"` exit with an error (use the Python node for
  those). Emulation still reports the configured hat's 10-bit range, so
  the app can be exercised against both widths.
- BLE is not supported; `"transport": "ble"` falls back to Wi-Fi with a
  warning.
- A failed ADC read is logged and the sample skipped instead of stopping
  the node.
