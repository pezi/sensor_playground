# TM1637 Clock Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a Grove 4-Digit Display
(TM1637) — the *actuator* variant of the interface. The app syncs `hh:mm`
to the display and the node reports the state it is actually showing
back. Once a time is set the node keeps the clock running on its own — it
advances the minute and blinks the colon locally, so the display stays a
working clock even when the app is away:

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"time": "HH:MM"}` | set the displayed time (24-hour; invalid values are ignored) |
| app → node | `{"brightness": 0..7}` | set the brightness (clamped; 0 is dimmest but still lit) |
| node → app | `{"time": "12:34", "brightness": 3}` | current state — on connect, after every accepted command, and on each minute rollover |
| node → app | `{"time": null, "brightness": 3}` | no time has been set yet (display shows `--:--`) |

The colon blink is local only and never generates a message, so state
traffic stays at one message per minute.

It serves a **WebSocket** (`ws://`, port 9132, `X-Api-Key` handshake
header) instead of HTTPS REST, plus UDP discovery on port 9133.

It is the Go counterpart of [`../../python/tm1637/`](../../python/tm1637/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Wiring (GPIO)

`interface` must be `"gpio"`: `clk_pin` / `dio_pin` are BCM numbers on
`/dev/gpiochip0` (set `gpio_chip` for boards where the header lives on
another chip). This also covers the **Seeed Grove Base Hat** — its
digital ports are wired straight to the Pi.

| Pi Pin | Grove Pin |
|--------|-----------|
| 3.3V or 5V | VCC |
| GND | GND |
| `clk_pin` (default BCM 5) | CLK (yellow) |
| `dio_pin` (default BCM 6) | DIO (white) |

The TM1637 speaks a proprietary two-wire protocol (start/stop conditions
like I2C, but LSB-first and without addresses) and has no minimum clock
speed, so the node bit-bangs it on two GPIO character-device lines — one
ioctl per transition paces the bus well below the chip's limit.
`/sys/class/gpio` is gone in Debian 13.

The chip acknowledges every byte by pulling DIO low; the node clocks
through that ACK slot without reading it (the line stays an output), which
the chip tolerates.

> The Arduino-based extension hats the Python node also refuses
> (`"interface": "hat"` — NanoHat Hub, GrovePi+) are **not** supported:
> one display frame needs ~50 line transitions and each hat
> `digital_write` is a full I2C transaction, far too slow for a display
> bus. The node refuses to start with a clear message.

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `gpio_chip` | `/dev/gpiochip0` | GPIO character device carrying the header pins. |
| `clk_pin` | `5` | BCM number of the GPIO driving the display clock line. |
| `dio_pin` | `6` | BCM number of the GPIO driving the display data line. |
| `brightness` | `3` | Brightness at start (0 dimmest … 7 brightest; 0 is still lit — the TM1637 has no off level below it). |

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_tm1637 .
cp config.example.json config.json    # edit: api_key, clk_pin, dio_pin
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_tm1637 .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then prints the displayed time instead of driving a panel,
on time/brightness changes and minute rollovers only. Works on any
machine (macOS/Windows included).

## Usage

```bash
./sensor_node_tm1637
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# state pushes + commands, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
# then type: {"time": "12:34"}
```

The current state prints on connect (`{"time": null, "brightness": 3}` on
a fresh start); after the command the display shows 12:34 with a blinking
colon and the node echoes the new state. A minute later it echoes `12:35`
on its own.

`go test` checks the segment encoding (digit patterns, the `--:--`
placeholder, the colon bit), the `"HH:MM"` command parser and the
brightness clamp against the Python node's tables.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-tm1637-go.service`.

## Deviations from the Python Node

- BLE transport falls back to Wi-Fi (see above).
- The display bus is bit-banged on GPIO character-device lines
  (`/dev/gpiochipN`) instead of gpiozero, so none of the Python node's
  pin-factory setup applies. The protocol bytes, segment patterns, ACK
  handling and blanking on exit match the Python node.
- `gpio_chip` selects the GPIO character device; the Python config has no
  such key. Everything else — protocol, config schema, clock/blink
  timing, emulation output — matches the Python node.
