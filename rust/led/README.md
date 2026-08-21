# LED Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface
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

It is the Rust counterpart of [`../../python/led/`](../../python/led/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).
  Over BLE the state arrives as notifies on the data characteristic
  and commands as binary writes on the command characteristic
  (`0x01 0x00` off, `0x01 0x01` on, `0x02` flip).

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

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace. On a non-Pi development host, build from this folder
normally:

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key, led_pin, button_pin
```

For a native Raspberry Pi build, `rustc 1.97.1` can crash with
`SIGSEGV`. Use the verified Rust 1.96.0 workaround and compile one job
at a time (see the [diagnosis and power checks](../bme680/README.md#native-raspberry-pi-builds)):

```bash
rustup toolchain install 1.96.0 --profile minimal
rustup override set 1.96.0
cargo build --release -j 1
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
../target/release/sensor_node_led
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
with the unit name `sensor-playground-led-rust.service`.
