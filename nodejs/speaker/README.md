# Speaker Node for Sensor Playground (Node.js)

This Node.js program implements the **actuator** variant of the Sensor
Playground sensor interface on single-board computers (Raspberry Pi & co.)
with a **Grove Speaker** — a small amplified loudspeaker on a digital pin,
driven with a square wave of the desired pitch. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and connects to its
WebSocket (port 9132, `X-Api-Key` handshake header).

It is the Node.js counterpart of [`../../python/speaker/`](../../python/speaker/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to the WebSocket with a warning (use the Python or Rust node for BLE).

## Protocol

```
app -> node   {"tone": {"freq": 440, "ms": 400}}   play one tone
              {"melody": true}                     play the built-in melody
              {"stop": true}                       silence
node -> app   {"freq": 440} / {"freq": 0}          what is sounding
```

The node owns the state: a tone ends on its own when its duration runs out,
so the app follows the node's reports rather than its own taps. The state
is pushed on connect, after every accepted command, and whenever a tone or
melody step ends. Tones outside 20-20000 Hz, or without a positive
duration, are ignored.

## Emulation only

**This port cannot make a sound.** Sounding a tone means toggling a GPIO
line up to 20000 times a second, and Node's event loop cannot hold that
pace — the same reason `dht11`, `sht11`, `ppd42ns`, `ultrasonic` and
`tm1637` are emulation-only here. The node runs the full protocol: the
WebSocket transport, discovery, command validation, and the timing of tones
and melodies are all complete, and it prints what would be sounding. Only
the pin is never driven.

Use the [Python](../../python/speaker/), [Rust](../../rust/speaker/) or
[Go](../../go/speaker/) node to actually drive a speaker. The node prints a
warning and continues in emulation when `config.json` asks for hardware.

## Wiring (for the other ports)

| Pi Pin | Grove Pin |
|--------|-----------|
| 5V (Pin 2) or 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 5 (Pin 29, `speaker_pin` in config) | SIG (yellow) |

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key
```

## Usage

```bash
node sensor_node.js
```

No TLS certificates: the WebSocket transport is plain `ws://`, authorized
with the `X-Api-Key` handshake header.

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# WebSocket handshake with the API key, then send a command:
#   {"tone": {"freq": 440, "ms": 400}}
# The node answers {"freq":440} and, 400 ms later, {"freq":0}.
```

Any WebSocket client works as long as it sends the `X-Api-Key` header; a
wrong or missing key is rejected with 401 on the handshake.

`npm test` checks the command handling and playback state: out-of-range and
malformed tones are ignored, the melody steps through its notes and falls
silent, and a stop cancels a running melody.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-speaker-node.service`.
