# Grove Dust Sensor (PPD42NS) Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
with a [Grove Dust Sensor](https://wiki.seeedstudio.com/Grove-Dust_Sensor/)
(Shinyei PPD42NS). The Sensor Playground app discovers this node via UDP
broadcast (port 9133) and polls it for data over HTTPS (port 9132,
`X-Api-Key` header); the app shows the value as a classified air-quality
card (Dylos bands) plus a live chart.

It is the Node.js counterpart of
[`../../python/ppd42ns/`](../../python/ppd42ns/) and speaks the identical
wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

> **Emulation only.** The PPD42NS signals particles as 10–90 ms LOW
> pulses whose occupancy must be accumulated from kernel edge
> timestamps — no Node.js GPIO path delivers those timestamps (a
> JavaScript polling loop adds milliseconds of jitter per edge, a large
> error on a 10 ms pulse). With `"emulation": false` the node prints an
> error and exits; use the Python, Go or Rust node for the real sensor.

## How the real sensor is measured

The Python, Go and Rust nodes accumulate the sensor's **low-pulse
occupancy (LPO)** over 30-second windows and convert the ratio with the
Nafis curve:

    ratio         = low_time / window_time * 100          (percent)
    concentration = 1.1·r³ − 3.8·r² + 520·r + 0.62        (pcs/0.01cf)

This port only emulates the resulting `dust` value.

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
sensor hardware — it then serves plausible generated readings crossing
several Dylos air-quality bands. Works on any machine (macOS/Windows
included). This is the only mode this port supports (see above).

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
{"sensor":"PPD42NS","host":"raspberrypi","dust":412.5}
```

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-ppd42ns-node.service`.

## Deviations from the Python Node

- **Real hardware is not supported** — this port runs in emulation mode
  only. Accumulating the PPD42NS's low-pulse occupancy needs
  kernel-timestamped GPIO edge events, which Node.js cannot access (see
  above); the Python, Go and Rust nodes drive the real sensor. There is
  therefore no warm-up phase: the emulated value is served immediately
  (the Python node answers 503 `{"error":"warming_up"}` until its first
  30-second window closes).
- BLE transport falls back to Wi-Fi (see above). The emulation formula
  and payloads match the Python node.
