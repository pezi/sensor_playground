# IMU 10DOF Sensor Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface
on single-board computers (Raspberry Pi & co.) with a
[Grove IMU 10DOF](https://wiki.seeedstudio.com/Grove-IMU_10DOF/) board
(MPU9250 + BMP280). The MPU9250 axes are reduced to roll, pitch and compass
heading angles plus the total acceleration magnitude (g-force); the BMP280
adds temperature and barometric pressure. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and then **streams** the
readings from a WebSocket (port 9132, `ws://`, `X-Api-Key` header on the
handshake) — the app streams motion sensors rather than polling them.
Readings are pushed every 250 ms while a client is connected.

It is the Go counterpart of [`../../python/imu10dof/`](../../python/imu10dof/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to the WebSocket with a warning (use the Python or Rust node for BLE).

The MPU9250/AK8963 driver is a port of the
[mpu9250-jmdev](https://pypi.org/project/mpu9250-jmdev/) Python driver
(the library the Python node uses): bypass mode, 8 g accelerometer and
16-bit / 100 Hz magnetometer full scales. The BMP280 is driven with the
Bosch datasheet integer algorithm, ported from the Python node's own
minimal driver — readings match the Python node.

> Roll/pitch come from the accelerometer and the heading from the raw
> magnetometer (not tilt-compensated), so the heading is most accurate when
> the board lies flat.

## Wiring (I2C, addresses 0x68 / 0x0C / 0x77)

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
go build -o sensor_node_imu10dof .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_imu10dof .   # Pi 3/4/5 (64-bit OS)
```

## Emulation

Set `"emulation": true` in `config.json` to run the node without any
hardware — it then serves plausible generated readings. Works on any
machine (macOS/Windows included).

## Usage

```bash
./sensor_node_imu10dof
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
# stream readings, e.g. with websocat:
websocat -H "X-Api-Key: your-sensor-api-key" ws://<ip>:9132
```

```json
{"sensor":"IMU10DOF","host":"raspberrypi","temperature":22.4,"pressure":1013.25,"roll":1.2,"pitch":-0.8,"heading":132.5,"gforce":1.01}
```

`go test` checks the accelerometer and magnetometer scaling against
hand-computed values and the BMP280 compensation against 200 golden
values generated with the Python node's own BMP280 driver.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-imu10dof-go.service`.

## Deviations from the Python node

- BLE is not supported (Wi-Fi/WebSocket fallback with a warning).
- A failed BMP280 read skips that push (and logs the error) instead of
  terminating the node; a failed accelerometer/magnetometer read yields
  zero axes, exactly like the mpu9250-jmdev reference driver.
