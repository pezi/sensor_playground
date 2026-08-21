# Grove Capacitive Moisture Sensor Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a **Grove Capacitive
Moisture Sensor (Corrosion-Resistant)** — an analog probe whose output
voltage falls as the soil gets wetter. It reports the soil moisture as a
percentage (JSON key `moisture`) mapped linearly between the two
calibration points in `config.json`, alongside the raw reading
(`adc`/`adcMax`) for calibrating them. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Node.js counterpart of [`../../python/moisture/`](../../python/moisture/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

## Needs a Grove Base Hat

The Raspberry Pi has no analog input, so the probe is read through the
Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04): plug the probe
into one of the hat's analog ports (A0-A7) and set `pin` to that channel.
Do not insert the probe deeper than the marked line — the electronics at
the top are not waterproof.
Enable I2C: `sudo raspi-config` → Interface Options → I2C.

> The Arduino-based hats the Python node also supports (`"nano"`,
> `"grovePlus"`) are **not** implemented in this port — use the Python
> node for those.

## Calibration

`moisture` is a `0`-`100` % value mapped linearly between two calibration
points in `config.json`:

| Key | Meaning | Default |
|-----|---------|---------|
| `adc_dry` | averaged raw ADC reading in dry air | `2600` |
| `adc_wet` | averaged raw ADC reading submerged in water | `1100` |

Probes and hats vary, so **calibrate yours**: the REST payload also
carries the averaged raw reading (`adc`, with its full scale `adcMax`).
Hold the probe in dry air, read `adc`, put the number into `adc_dry`;
submerge it up to the marked line in a glass of water and do the same
for `adc_wet`. Values outside the calibrated span clamp to 0/100 %.

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, pin, adc_dry, adc_wet
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves a plausible watering-and-drying cycle. Works
on any machine (macOS/Windows included).

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
{"sensor":"MOISTURE","host":"raspberrypi","moisture":63,"adc":1820,"adcMax":4095}
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-moisture-node.service`.
