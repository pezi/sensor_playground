# Grove Ultrasonic Ranger Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a
[Grove Ultrasonic Ranger](https://wiki.seeedstudio.com/Grove-Ultrasonic_Ranger/)
(40 kHz sonar, 2 cm – 3.5 m) — a *push* node like the VL53L0X: it
measures continuously and pushes one JSON message over a **WebSocket**
(`ws://`, port 9132, `X-Api-Key` handshake header) whenever the distance
changes, or at least once per second as a heartbeat, plus UDP discovery
on port 9133.

It is the Go counterpart of
[`../../python/ultrasonic/`](../../python/ultrasonic/) and speaks the
identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

> **Single SIG pin.** Unlike the common HC-SR04 with separate TRIG/ECHO
> pins, the Grove ranger multiplexes both on one wire: the node drives a
> trigger pulse, switches the pin to input and times the echo pulse the
> module answers with. The echo is timed via kernel-timestamped GPIO edge
> events (character device, `/dev/gpiochipN`) — a userspace polling loop
> would add milliseconds of jitter, and one millisecond of pulse error is
> 17 cm of distance error. There is no extension-hat option: the
> Arduino-based hats are polled over I2C and cannot time the echo.

## Protocol

| Message | Meaning |
|---------|---------|
| `{"distance": 234}` | distance in millimeters |
| `{"distance": null}` | no echo / target out of range |

## Supported Platforms

| Board | Notes |
|-------|-------|
| Raspberry Pi | `gpio_chip: 0`; the SIG pin is a plain GPIO |

### Wiring

| Pi Pin | Grove Pin |
|--------|-----------|
| 5V (Pin 2) or 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 4 (Pin 7, `sig_pin` in config) | SIG (yellow) |

> The module runs on 3.3–5V. When powering from 5V, prefer 3.3V if you
> want to be conservative about the SIG level on the Pi's input.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_ultrasonic .
cp config.example.json config.json    # edit: api_key, sig_pin
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_ultrasonic .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
ranger hardware — it then simulates a target sweeping between 200 and
2000 mm. Works on any machine (macOS/Windows included).

## Usage

```bash
./sensor_node_ultrasonic
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Wave a hand in front of the ranger — `{"distance": ...}` messages track
it; point it into open space and `{"distance": null}` appears.

`go test` checks the pulse-width → distance conversion and the publish
policy (change / echo flip / heartbeat) against the Python node's
formulas.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-ultrasonic-go.service`.

## Deviations from the Python Node

- BLE transport falls back to Wi-Fi (see above).
- The echo is timed via GPIO character-device edge events instead of
  lgpio alerts — the timestamps come from the same kernel machinery. The
  SIG line is re-requested per measurement (output for the trigger, then
  edge-event input for the echo), like the Python node's lgpio claim
  flips. Everything else — payloads, publish policy, timing, emulation —
  matches the Python node.
