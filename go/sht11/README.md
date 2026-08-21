# SHT11 Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a Sensirion SHT11 sensor (temperature, humidity) — the
classic SHT1x family. The Sensor Playground app discovers this node via
UDP broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header).

It is the Go counterpart of [`../../python/sht11/`](../../python/sht11/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The SHT10, SHT11 and SHT15 differ only in calibration accuracy (±0.5 /
±0.4 / ±0.3 °C typical) and speak the same proprietary **two-wire
protocol** (SCK + bidirectional DATA). It resembles I2C but is *not*
I2C — the sensor cannot share an I2C bus with other devices. Set
`sensor_name` in `config.json` to the chip on your board so the app
shows the right name.

> The protocol is bit-banged directly on two GPIOs via the character
> device (`/dev/gpiochipN`): the bus is fully master-clocked with no
> minimum speed, so the pace of one GPIO call per clock edge is
> harmless — the sensor simply waits between edges (unlike the DHT11,
> whose reply timing a userspace loop cannot follow). There is no
> extension-hat option. The sensor must not be measured more than ~10%
> of the time or it heats itself; the node reads at most every two
> seconds and serves the cached values, and a failed read serves the
> last good reading for up to 30 seconds.

## Supported Platforms

| Board | Notes |
|-------|-------|
| Raspberry Pi | `gpio_chip: 0`; the two pins are plain GPIOs |

### Wiring (two-wire, NOT I2C)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VDD |
| GND (Pin 6) | GND |
| GPIO 4 (Pin 7, `data_pin` in config) | DATA |
| GPIO 5 (Pin 29, `sck_pin` in config) | SCK |

> The DATA line needs a pull-up resistor (~10 kΩ to 3.3V); most breakout
> boards carry one, and the node additionally enables the Pi's internal
> pull-up. The temperature conversion constant assumes a 3.3V supply.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_sht11 .
cp config.example.json config.json    # edit: api_key, data_pin, sck_pin
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_sht11 .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
./sensor_node_sht11
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"SHT11","host":"raspberrypi","temperature":22.4,"humidity":45.1}
```

`go test` checks the temperature/humidity conversion formulas (3.3V
temperature constant, humidity polynomial + temperature compensation,
0..100 %RH clamp). There is no CRC to test: like the Python node, the
driver ends the transfer before the CRC byte.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-sht11-go.service`.

## Deviations from the Python Node

- BLE is not supported; `"transport": "ble"` falls back to Wi-Fi.
- The DATA line direction flips re-request the line on the GPIO
  character device instead of lgpio claims — the same open-drain
  emulation (input with pull-up for a 1/released, output-low for a 0).
  Protocol, conversion formulas, 2 s throttle / 30 s cache and emulation
  match the Python node.
