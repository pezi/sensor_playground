# TM1637 Clock Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a
[Grove 4-Digit Display](https://wiki.seeedstudio.com/Grove-4-Digit_Display/)
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

It is the Node.js counterpart of
[`../../python/tm1637/`](../../python/tm1637/) and speaks the identical
wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

> **Emulation only.** The TM1637 speaks a proprietary two-wire protocol
> (start/stop conditions like I2C, but LSB-first and without addresses)
> that has to be bit-banged: one frame is ~50 individually driven line
> transitions on two GPIOs. Node.js has no GPIO path for that on
> Debian 13 — `node-libgpiod` does not build against libgpiod 2.x, and
> the `gpioset` CLI fallback the LED node uses spawns a process per level
> change, which cannot carry a bus. With `"emulation": false` the node
> prints an error and exits; use the Python, Go or Rust node for a real
> display.

## Configuration

| Key | Default | Meaning |
|-----|---------|---------|
| `clk_pin` | `5` | BCM number of the GPIO driving the display clock line (kept for schema parity; unused in this port). |
| `dio_pin` | `6` | BCM number of the GPIO driving the display data line (kept for schema parity; unused in this port). |
| `brightness` | `3` | Brightness at start (0 dimmest … 7 brightest; 0 is still lit — the TM1637 has no off level below it). |

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, set "emulation": true
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then prints the displayed time instead of driving a panel,
on time/brightness changes and minute rollovers only. Works on any
machine (macOS/Windows included). This is the only mode this port
supports (see above).

## Usage

```bash
node sensor_node.js
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
a fresh start); after the command the node logs the virtual display and
echoes the new state. A minute later it echoes `12:35` on its own.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-tm1637-node.service`.

## Deviations from the Python Node

- **Real hardware is not supported** — this port runs in emulation mode
  only. Bit-banging the TM1637's two-wire bus needs per-transition GPIO
  writes that Node.js cannot deliver (see above); the Python, Go and Rust
  nodes drive the real display. `interface`, `clk_pin` and `dio_pin` are
  accepted for schema parity but unused.
- BLE transport falls back to Wi-Fi (see above). The protocol, state
  ownership, clock/blink timing, brightness clamp and emulation output
  match the Python node.
