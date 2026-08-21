# Grove Ultrasonic Ranger Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a
[Grove Ultrasonic Ranger](https://wiki.seeedstudio.com/Grove-Ultrasonic_Ranger/)
(40 kHz sonar, 2 cm – 3.5 m) — a *push* node like the VL53L0X: it
measures continuously and pushes one JSON message over a **WebSocket**
(`ws://`, port 9132, `X-Api-Key` handshake header) whenever the distance
changes, or at least once per second as a heartbeat, plus UDP discovery
on port 9133.

It is the Node.js counterpart of
[`../../python/ultrasonic/`](../../python/ultrasonic/) and speaks the
identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

> **Emulation only.** The Grove ranger multiplexes trigger and echo on
> one SIG pin, and the echo pulse must be timed with kernel edge
> timestamps — one millisecond of pulse error is 17 cm of distance
> error, and no Node.js GPIO path delivers those timestamps (a
> JavaScript polling loop adds milliseconds of jitter). With
> `"emulation": false` the node prints an error and exits; use the
> Python, Go or Rust node for the real sensor.

## Protocol

| Message | Meaning |
|---------|---------|
| `{"distance": 234}` | distance in millimeters |
| `{"distance": null}` | no echo / target out of range |

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
ranger hardware — it then simulates a target sweeping between 200 and
2000 mm. Works on any machine (macOS/Windows included). This is the only
mode this port supports (see above).

## Usage

```bash
node sensor_node.js
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

`{"distance": ...}` messages sweep between 200 and 2000 mm, with the
occasional `{"distance": null}` (simulated lost echo).

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-ultrasonic-node.service`.

## Deviations from the Python Node

- **Real hardware is not supported** — this port runs in emulation mode
  only. Timing the ranger's echo needs kernel-timestamped GPIO edge
  events, which Node.js cannot access (see above); the Python, Go and
  Rust nodes drive the real sensor.
- BLE transport falls back to Wi-Fi (see above). The emulation formula,
  payloads and publish policy match the Python node.
