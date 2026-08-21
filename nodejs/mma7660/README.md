# MMA7660 Accelerometer Node for Sensor Playground (Node.js)

This Node.js program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a
[Grove 3-Axis Digital Accelerometer ±1.5g](https://wiki.seeedstudio.com/Grove-3-Axis_Digital_Accelerometer-1.5g/)
(MMA7660FC). The raw axes are converted into roll and pitch angles plus the
total acceleration magnitude (g-force). The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and then **streams** the
readings from a WebSocket (port 9132, `ws://`, `X-Api-Key` header on the
handshake) — the app streams accelerometers rather than polling them;
readings are pushed every 250 ms.

It is the Node.js counterpart of [`../../python/mma7660/`](../../python/mma7660/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The MMA7660 is driven directly over I2C like the Python node's smbus2
access (6-bit two's-complement axes, 21.33 counts per g) — no extra
driver package needed beyond the optional `i2c-bus`.

## Wiring (I2C, address 0x4C)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C. Other boards:
set `i2c_bus` (Raspberry Pi `1`, NanoPi `0`, Banana Pi `2`).

## Setup

Install Node.js ≥ 18 (see [`../bme680/README.md`](../bme680/README.md)),
then set up the node. The shared [`../common/`](../common) folder must be
deployed next to this node folder (like the Python nodes):

```bash
npm install
cp config.example.json config.json    # edit: api_key, i2c_bus
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
node sensor_node.js
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# stream readings, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

```json
{"sensor":"MMA7660","host":"raspberrypi","roll":2.1,"pitch":-0.8,"gforce":1.01}
```

`npm test` checks the 6-bit two's-complement axis decoding, the
counts-per-g conversion and the roll/pitch/g-force math.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-mma7660-node.service`.

## Deviations from the Python node

- BLE is not supported (see above).
- A failed sensor read is logged and that push skipped; the Python node
  lets the exception end the process.
