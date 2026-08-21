# SHT11 Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface on
single-board computers with a Sensirion SHT11 sensor (temperature,
humidity) — the classic SHT1x family. The Sensor Playground app discovers
this node via UDP broadcast (port 9133) and polls it for data over HTTPS
(port 9132, `X-Api-Key` header).

It is the Node.js counterpart of
[`../../python/sht11/`](../../python/sht11/) and speaks the identical
wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The SHT10, SHT11 and SHT15 differ only in calibration accuracy (±0.5 /
±0.4 / ±0.3 °C typical) and speak the same proprietary **two-wire
protocol** (SCK + bidirectional DATA). It resembles I2C but is *not*
I2C — the sensor cannot share an I2C bus with other devices. Set
`sensor_name` in `config.json` to the chip on your board so the app
shows the right name.

> **Emulation only.** The two-wire protocol is not timing-critical — the
> bus is fully master-clocked, so any pace works — but it needs a
> bidirectional, open-drain DATA line whose state survives the hundreds
> of clock edges of one transaction: released (input with pull-up) for a
> 1, actively pulled LOW for a 0, sampled while the sensor answers.
> Node.js has no GPIO character-device binding on Debian 13
> (`/sys/class/gpio` is gone and `node-libgpiod` does not build against
> libgpiod 2.x), and the `gpioset`/`gpioget` CLIs release the line when
> they exit — so the handshake collapses after the very first edge.
> There is no extension-hat option either: the Arduino-based hats speak
> I2C, and this sensor does not. With `"emulation": false` the node
> prints an error and exits; use the Python, Go or Rust node for the
> real sensor.

## How the real sensor is read

The Python, Go and Rust nodes bit-bang the protocol on two GPIOs: a
transmission-start pattern (DATA falls and rises while SCK is high), the
command byte (`0x03` temperature, `0x05` humidity) MSB first, the
sensor's ACK on the ninth clock, then — once the sensor pulls DATA low to
signal "measurement done" — two data bytes MSB first. The raw values
become readings via the datasheet V5 formulas:

    T  = -39.66 + 0.01 * raw                              (14 bit, 3.3V)
    RH = -2.0468 + 0.0367 * raw - 1.5955e-6 * raw²
         + (T - 25) * (0.01 + 0.00008 * raw)              (12 bit)

The sensor must not be measured more than ~10% of the time or it heats
itself, so those nodes read at most every two seconds and serve the
cached values, and a failed read serves the last good reading for up to
30 seconds. This port only emulates the resulting `temperature` /
`humidity` values.

## Supported Platforms

| Board | Notes |
|-------|-------|
| any machine | emulation only (macOS/Windows/Linux) |

### Wiring (two-wire, NOT I2C — for the Python/Go/Rust nodes)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VDD |
| GND (Pin 6) | GND |
| GPIO 4 (Pin 7, `data_pin` in config) | DATA |
| GPIO 5 (Pin 29, `sck_pin` in config) | SCK |

> The DATA line needs a pull-up resistor (~10 kΩ to 3.3V); most breakout
> boards carry one, and those nodes additionally enable the Pi's internal
> pull-up. The temperature conversion constant assumes a 3.3V supply.

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, set "emulation": true
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
sensor hardware — it then serves plausible generated readings. Works on
any machine (macOS/Windows included). This is the only mode this port
supports (see above).

## SSL Certificates

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

`cert.pem`/`key.pem` are resolved relative to the working directory and
excluded from Git.

## Usage

```bash
node sensor_node.js
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

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-sht11-node.service`.

## Deviations from the Python Node

- **Real hardware is not supported** — this port runs in emulation mode
  only. Bit-banging the SHT1x two-wire protocol needs a persistent,
  bidirectional GPIO line, which Node.js cannot get on Debian 13 (see
  above); the Python, Go and Rust nodes drive the real sensor.
  `data_pin`, `sck_pin` and `gpio_chip` are therefore accepted in
  `config.json` (so the same file works for all ports) but unused, and
  the 2 s throttle / 30 s cache policy has nothing to cache: the
  emulated reading never fails, so the node never answers 503
  `{"error":"sensor_read_failed"}`.
- BLE transport falls back to Wi-Fi (see above).
- `sensor_name` still selects the reported name (`SHT10`/`SHT11`/
  `SHT15`). The emulation formulas, payload keys and config schema match
  the Python node.
