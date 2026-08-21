# Relay Node for Sensor Playground (Rust)

> [!WARNING]
> ### ⚡ Mains voltage kills — read this before wiring the load side
>
> These modules are rated **10 A at 250 VAC**. The contact side is built to
> switch **230 V / 120 V mains**, which is lethal to touch and will start a
> fire if it is wired or enclosed badly.
>
> - **De-energize first.** Pull the plug and switch off the breaker before you
>   touch a screw terminal — "relay off" in the app is *not* de-energized.
> - **Treat every contact as live while the board is powered**, whatever the
>   app shows. A welded contact, a mis-numbered channel, a crash or a stray
>   command can leave a channel closed while the app says open.
> - **Enclose the module before powering it.** Bare screw terminals and 230 V
>   do not belong on a desk.
> - **Keep the two sides apart.** Mains on the contact side only — never
>   running alongside the low-voltage wiring to the Raspberry Pi, and never
>   bridging the isolation slot in the board.
> - **This is not a safety device.** A relay is not an isolator, an emergency
>   stop or a lockout. Never rely on it to make anything safe to work on.
> - **Fixed mains wiring is a licensed trade in most countries.** If any of
>   this is unfamiliar, switch a low-voltage load instead — a 12 V lamp or fan
>   behaves identically in the app.

An actuator node for Grove SPDT relay modules with 1, 2 or 4 channels. It is
the Rust counterpart of `../../python/relay/`: the app switches a channel and
the node reports the state of the complete board.

## Protocol

| Direction | Message | Meaning |
|---|---|---|
| app → node | `{"ch":0,"on":true}` | switch channel 0 on |
| app → node | `{"ch":0,"toggle":true}` | toggle channel 0 |
| app → node | `{"all":false}` | switch every channel off |
| node → app | `{"channels":2,"relay":[true,false]}` | complete board state |

Wi-Fi uses an authenticated WebSocket on port 9132 plus UDP discovery on
9133. Discovery includes `"channels"`. With `"transport":"ble"`, state is
notified as the same JSON and commands use the ESP32/Python binary protocol:

| BLE packet | Meaning |
|---|---|
| `01 <ch> 00/01` | set one channel |
| `02 <ch>` | toggle one channel |
| `03 00/01` | set every channel |

## Hardware

- `"interface":"gpio"`: Grove 1-/2-channel modules, one `relay_pins`
  GPIO character-device offset per channel.
- `"interface":"i2c"`: Grove 4-channel SPDT relay. The node writes the
  native `0x10 <mask>` command to address `0x11` directly.
- `"interface":"gpio"` also drives the common 4-channel
  ["4 Channel 5V Relay Module"](https://www.sunfounder.com/products/4channel-relay-shield)
  boards (SunFounder & clones): they are *not* I2C, just four input pins, so
  list four entries in `relay_pins`. `relay_pins` takes 1-8 lines in total.
  These boards are usually **active LOW** — set `"relay_active_low": true`.
  The node opens every channel at startup, so if the relays click closed when
  it starts, that flag is the wrong way round. They want 5 V and 15-20 mA per
  channel; if a channel will not switch reliably from 3.3 V logic, give the
  coils their own 5 V supply (JD-VCC jumper removed, GND shared).
- Arduino-based GrovePi+ and NanoHat Hub interfaces are Python-only.

The modules use 5V VCC and a shared ground. GPIO defaults to line offsets 5
and 6 on `/dev/gpiochip0`. For I2C, connect SDA to GPIO 2, SCL to GPIO 3 and
enable I2C with `raspi-config`.

## Setup

```bash
cp config.example.json config.json
# Edit api_key and the interface/pins or I2C settings.
cargo run --release -p sensor-playground-relay
```

Set `"emulation":true` to switch virtual relays while preserving the
configured channel count.

## Configuration

| Key | Default | Meaning |
|---|---|---|
| `sensor_name` | `RELAY` | advertised type |
| `interface` | `gpio` | `gpio` or `i2c` |
| `gpio_chip` | `/dev/gpiochip0` | GPIO character device |
| `relay_pins` | `[5,6]` | GPIO line offsets, one per channel |
| `relay_active_low` | `false` | logical output polarity |
| `channels` | `4` | I2C board width, 1–8 |
| `i2c_bus` | `1` | `/dev/i2c-N` bus number |
| `i2c_address` | `0x11` | I2C board address |
| `transport` | `wifi` | `wifi` or `ble` |
| `emulation` | `false` | use virtual relay outputs |

## Test

```bash
cargo test -p sensor-playground-relay
```

The process switches all channels off on clean shutdown.

## Safety

**These modules switch mains-level loads (10 A at 250 VAC / 30 VDC).** Read the
warning at the top of this file before you wire anything to the contact side.

In short: de-energize the load and the circuit before touching a terminal,
treat the contacts as live whenever the board is powered, keep mains wiring on
the contact side and away from the low-voltage side to the Raspberry Pi, and
enclose the module before powering it. The node opens every channel on a clean
shutdown, but a crash, a power cut or a pulled cable leaves the contacts where
they happened to be — so never treat the relay as an isolator, an emergency
stop or a lockout for work on the load.

If mains wiring is not something you are comfortable and qualified to do, use a
low-voltage load instead: the node behaves identically with a 12 V lamp, and
in many countries fixed mains work is legally reserved for electricians.

