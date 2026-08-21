# LED Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with an LED and an optional local push button — the *actuator* variant of
the interface. The app sends a switch command and the node reports the
resulting state back, because the button can also toggle the LED:

| Direction | Message | Meaning |
|-----------|---------|---------|
| app → node | `{"led": true}` / `{"led": false}` | switch on / off |
| app → node | `{"toggle": true}` | flip |
| node → app | `{"led": true\|false}` | current state (on connect and after every change) |

It serves a **WebSocket** (`ws://`, port 9132, `X-Api-Key` handshake
header) instead of HTTPS REST, plus UDP discovery on port 9133.

It is the Node.js counterpart of [`../../python/led/`](../../python/led/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Wiring (GPIO)

LED (with series resistor) on `led_pin`, optional button on `button_pin`
(BCM numbers on `/dev/gpiochip0`; set `gpio_chip` for boards where the
header lives on another chip). `led_active_low: true` if the LED lights
when the pin is driven LOW; `button_active_low: true` (default) uses the
internal pull-up and a button that pulls to GND. `button_pin: null` makes
an LED-only node.

The GPIO lines are driven through the character device
(`/dev/gpiochipN`) — `/sys/class/gpio` is gone in Debian 13. The
Arduino-based extension hats the Python node also supports
(`"interface": "hat"`) are **not** implemented in this port.

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, led_pin, button_pin
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

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
# then type: {"led": true}
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-led-node.service`.

## GPIO backends

On systems with libgpiod 1.x (Debian 12) the optional `node-libgpiod`
package drives LED and button natively. On libgpiod 2.x systems
(Debian 13 "trixie") that package cannot build; the node then falls back
to the `gpioset` CLI (`sudo apt install gpiod`) for the LED — the
**button is not supported** on this fallback (use the Python, Go or Rust
node for a button on Debian 13).
