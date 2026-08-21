# Digital Contact Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) — one generic **push** node
for the simple two-state Grove/BakeBit digital sensors. It polls a
debounced input and tells the Sensor Playground app whenever the state
changes:

| Direction | Message | Meaning |
|-----------|---------|---------|
| node → app | `{"active": true}` | sensor triggered |
| node → app | `{"active": false}` | sensor released |

It serves a **WebSocket** (`ws://`, port 9132, `X-Api-Key` handshake
header) instead of HTTPS REST, plus UDP discovery on port 9133. On
connect the node first sends the current state.

It is the Go counterpart of
[`../../python/digital_contact/`](../../python/digital_contact/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Supported sensors

Set `sensor_name` and `active_low` in `config.json` for your sensor:

| Sensor | `sensor_name` | `active_low` | Wiki |
|--------|---------------|--------------|------|
| Button | `"BUTTON"` | `true` | https://wiki.seeedstudio.com/Grove-Button/ |
| Hall sensor | `"HALL"` | `true` | https://wiki.seeedstudio.com/Grove-Hall_Sensor/ |
| Magnetic switch | `"MAGSWITCH"` | `false` | https://wiki.seeedstudio.com/Grove-Magnetic_Switch/ |
| PIR motion | `"PIR"` | `false` | https://wiki.seeedstudio.com/Grove-PIR_Motion_Sensor/ |
| Vibration (SW-420) | `"VIBRATION"` | `true` | https://wiki.seeedstudio.com/Grove-Vibration_Sensor_SW-420/ |
| Line Finder | `"LINEFINDER"` | `false` * | https://wiki.seeedstudio.com/Grove-Line_Finder/ |

`active_low: true` means the sensor pulls the signal line LOW when active.

> \* The Line Finder is an infrared reflectance detector (TCRT5000 plus
> comparator): it sees a dark line against a bright surface a few millimetres
> below it — the classic line-following robot sensor. Its output polarity
> differs between board revisions, and the on-board potentiometer sets the
> black/white *threshold* rather than the direction. Hold the sensor over the
> line and check the app: if it reads "Line detected" over the bright surface
> instead, flip `active_low`.

The node enables the matching internal pull resistor so the idle state is
defined: a pull-up for `active_low: true` (idles HIGH) and a pull-down for
`active_low: false` (idles LOW). The pull-down matters for the reed-based
magnetic switch, which only connects the line to VCC while the magnet holds
it shut and floats otherwise.

## Wiring (GPIO)

Sensor signal on `pin` (BCM number on `/dev/gpiochip0`; set `gpio_chip`
for boards where the header lives on another chip). This also covers the
Seeed Grove Base Hat — its digital ports are wired straight to the Pi.

The GPIO line is read through the character device (`/dev/gpiochipN`) —
`/sys/class/gpio` is gone in Debian 13. The Arduino-based extension hats
the Python node also supports (`"interface": "hat"`) are **not**
implemented in this port.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_digital_contact .
cp config.example.json config.json    # edit: api_key, sensor_name, active_low, pin
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_digital_contact .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
./sensor_node_digital_contact
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# state pushes, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

Trigger the sensor; each change prints as a JSON line
(`{"active": true}` / `{"active": false}`). On connect the node first
sends the current state.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-digital_contact-go.service`.

## Deviations from the Python node

- `"interface": "hat"` (Arduino-based extension hats) is not implemented;
  the node exits with an error. The `hat_type` / `i2c_bus` config keys are
  therefore gone, and `gpio_chip` is new.
- BLE is not supported; `"transport": "ble"` falls back to Wi-Fi with a
  warning.
- The GPIO is read through the character device instead of gpiozero — the
  pull-resistor and polarity semantics are identical.
