# MPU6050 IMU Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with an MPU6050 6-axis IMU
(accelerometer + gyroscope). The accelerometer axes are converted into
roll and pitch angles plus the total acceleration magnitude (g-force);
the on-die temperature is reported as well. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and then **streams** the
readings from a WebSocket (port 9132, `ws://`, `X-Api-Key` header on the
handshake) — the app streams accelerometers rather than polling them;
readings are pushed every 250 ms.

It is the Go counterpart of [`../../python/mpu6050/`](../../python/mpu6050/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The MPU6050 is driven directly over I2C like the Python node's smbus2
access (wake from sleep, 14-byte burst read, 16384 LSB/g at the default
±2 g range) — no extra driver package needed. The temperature is measured
on the chip die and runs slightly warm.

## Wiring (I2C, address 0x68)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C. Other boards:
set `i2c_bus` (Raspberry Pi `1`, NanoPi `0`, Banana Pi `2`).

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_mpu6050 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_mpu6050 .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
./sensor_node_mpu6050
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# stream readings, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

```json
{"sensor":"MPU6050","host":"raspberrypi","temperature":36.2,"roll":2.1,"pitch":-0.8,"gforce":1.01}
```

`go test` checks the big-endian 16-bit decoding, the LSB-per-g
conversion (incl. negative values), the die-temperature formula and the
roll/pitch/g-force math.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-mpu6050-go.service`.

## Deviations from the Python node

- BLE is not supported (see above).
- A failed sensor read is logged and that push skipped; the Python node
  lets the exception end the process.
