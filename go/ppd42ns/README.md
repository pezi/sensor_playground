# Grove Dust Sensor (PPD42NS) Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a
[Grove Dust Sensor](https://wiki.seeedstudio.com/Grove-Dust_Sensor/)
(Shinyei PPD42NS). The Sensor Playground app discovers this node via UDP
broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header); the app shows the value as a classified air-quality
card (Dylos bands) plus a live chart.

It is the Go counterpart of [`../../python/ppd42ns/`](../../python/ppd42ns/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## How the value is measured

The PPD42NS pulls its output pin LOW while particles scatter light inside
its chamber (pulses of roughly 10–90 ms). The node timestamps both edges
via kernel-timestamped GPIO edge events (character device,
`/dev/gpiochipN`) and accumulates the **low-pulse occupancy (LPO)** over
30-second windows:

    ratio         = low_time / window_time * 100          (percent)
    concentration = 1.1·r³ − 3.8·r² + 520·r + 0.62        (pcs/0.01cf)

(the Nafis curve, https://www.howmuchsnow.com/arduino/airquality/grovedust/).

- **The first reading appears after the first full 30-second window.**
  Until then the REST endpoint answers 503 — that is warm-up, not a fault.
- Shinyei recommends letting the sensor stabilize for ~3 minutes after
  power-on before trusting the values.

## Direct GPIO only

The pin is read directly via GPIO edge events — there is **no
extension-hat option**: the Arduino-based hats are polled over I2C and
cannot timestamp 10–90 ms pulses. `pin` in `config.json` is the BCM GPIO
number; `gpio_chip` selects `/dev/gpiochipN` (Raspberry Pi: `0`).

### Wiring

| PPD42NS pin | Connect to |
|-------------|------------|
| 1 (GND, black) | GND |
| 3 (5V, red) | **5 V** (the heater draws ~90 mA — do not use 3V3) |
| 4 (P1 output, yellow) | voltage divider → GPIO (default GPIO 17) |

The output swings up to ~4.5 V, which is **not 3.3 V-safe**: pass it through a
voltage divider (e.g. 10 kΩ from the sensor output to the GPIO, 20 kΩ from
the GPIO to GND ⇒ 4.5 V → 3.0 V). Mount the sensor vertically — its chamber
relies on the heater's convection air flow.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_ppd42ns .
cp config.example.json config.json    # edit: api_key, pin
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_ppd42ns .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
sensor hardware — it then serves plausible generated readings crossing
several Dylos air-quality bands. Works on any machine (macOS/Windows
included).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
./sensor_node_ppd42ns
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"PPD42NS","host":"raspberrypi","dust":412.5}
```

During the first 30 seconds after start it returns 503 instead — the
first LPO window has not completed yet.

`go test` checks the ratio → concentration conversion (Nafis curve)
against golden values from the Python node's formula, plus the warm-up
and clean-window behavior.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-ppd42ns-go.service`.

## Deviations from the Python Node

- BLE transport falls back to Wi-Fi (see above).
- The warm-up 503 body is `{"error":"sensor_read_failed"}` (the shared
  Go transport's standard error), where the Python node answers
  `{"error":"warming_up"}`. Status code and timing are identical; only
  the machine-readable reason differs.
- The edges are timestamped by the kernel's GPIO character-device edge
  events instead of gpiozero callbacks — the same accumulation and window
  math on better timestamps (no userspace callback jitter).
- `gpio_chip` is a new config key selecting `/dev/gpiochipN` (default
  `0`, right for the Raspberry Pi). Everything else — payloads, window
  logic, stuck-low saturation, emulation — matches the Python node.
