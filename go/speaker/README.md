# Speaker Node for Sensor Playground (Go)

This Go program implements the **actuator** variant of the Sensor
Playground sensor interface on single-board computers (Raspberry Pi & co.)
with a **Grove Speaker** — a small amplified loudspeaker on a digital pin,
driven with a square wave of the desired pitch. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and connects to its
WebSocket (port 9132, `X-Api-Key` handshake header).

It is the Go counterpart of [`../../python/speaker/`](../../python/speaker/)
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

## How the tone is produced

The pin gets a 50%-duty square wave from a goroutine that toggles the line
and sleeps half a period. That is the same approach as the Python node,
whose `lgpio.tx_pwm` is itself a software PWM thread.

> Timing comes from the scheduler, so the pitch is only as steady as the
> system's sleep granularity — expect audible jitter under load. Good
> enough for a beeper; it is not a music synthesiser.

## Wiring

| Pi Pin | Grove Pin |
|--------|-----------|
| 5V (Pin 2) or 3.3V (Pin 1) | VCC (red) |
| GND (Pin 6) | GND (black) |
| GPIO 5 (Pin 29, `speaker_pin` in config) | SIG (yellow) |

`gpio_chip` selects `/dev/gpiochipN` (`0` on a Raspberry Pi).

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_speaker .
cp config.example.json config.json    # edit: api_key, speaker_pin
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_speaker .   # Pi 3/4/5 (64-bit OS)
```

The user running the node needs access to the GPIO character device (the
`gpio` group on Raspberry Pi OS).

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it prints the tones instead of sounding them, and the WebSocket
protocol behaves exactly as it does with a speaker attached. Works on any
machine (macOS/Windows included).

## Usage

```bash
./sensor_node_speaker
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

`go test` checks the command handling and playback state: out-of-range and
malformed tones are ignored, the melody steps through its notes and falls
silent, and a stop cancels a running melody.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-speaker-go.service`.
