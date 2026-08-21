# Rotary Angle Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface
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

It is the Rust counterpart of
[`../../python/rotary_angle/`](../../python/rotary_angle/)
and speaks the identical wire protocol.

- BLE GATT server (`"transport": "ble"`), like the Python node and the
  ESP32 sketch — Linux only, run as **root** (kernel advertising
  workaround, see [`../bme680/README.md`](../bme680/README.md)).
  Over BLE each movement arrives as a notify on the data characteristic;
  the node takes no commands, so there is no command characteristic.

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

Install Rust and the build prerequisites as described in
[`../bme680/README.md`](../bme680/README.md). The nodes form one Cargo
workspace. On a non-Pi development host, build from this folder
normally:

```bash
cargo build --release
cp config.example.json config.json    # edit: api_key, pin
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
hardware — it then sweeps slowly from end to end and back, as if someone
were turning the knob. Works on any machine (macOS/Windows included).

## Usage

```bash
../target/release/sensor_node_rotary_angle
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

The deadband/end-pinning logic is covered by unit tests:
`cargo test -p sensor-playground-rotary_angle`.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-rotary_angle-rust.service`.

## Deviations from the Python node

- Only the Grove Base Hat (`"hat_type": "grove"`) is implemented;
  `"nano"` / `"grovePlus"` exit with an error (use the Python node for
  those). Emulation still reports the configured hat's 10-bit range, so
  the app can be exercised against both widths.
- A failed ADC read is skipped (no message goes out) instead of stopping
  the node.
