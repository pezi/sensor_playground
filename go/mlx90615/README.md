# MLX90615 Infrared Thermometer Node for Sensor Playground (Go)

This Go program implements the Sensor Playground sensor interface on
single-board computers (Raspberry Pi & co.) with a Grove Digital Infrared
Temperature Sensor (**MLX90615**). It measures the **object temperature**
of whatever is in the sensor's field of view — without contact — plus the
**ambient temperature** of the sensor itself. The Sensor Playground app
discovers this node via UDP broadcast (port 9133) and polls it for data
over HTTPS (port 9132, `X-Api-Key` header).

It is the Go counterpart of [`../../python/mlx90615/`](../../python/mlx90615/)
and speaks the identical wire protocol.

- BLE is **not** supported in this port; `"transport": "ble"` falls back
  to Wi-Fi with a warning (use the Python or Rust node for BLE).

The driver is a straight port of the register access in the Python node
(smbus2): word reads from RAM register `0x26` (ambient) and `0x27`
(object), temperature = raw × 0.02 K − 273.15, bit 15 set marks an error —
readings match the Python node.

> **Repeated start.** The MLX90615 is a strict SMBus part: the register
> address and the data read must be one transfer with a repeated start. A
> plain write-then-read (what the other I2C nodes here do) puts a stop
> condition in between and the sensor aborts, so this node reads through
> the kernel's SMBus ioctl — the same call `smbus2`'s `read_word_data()`
> makes in the Python node.

## Readings

| JSON key (REST) | Meaning |
|-----------------|---------|
| `temperature` | the sensor's own ambient temperature, °C |
| `objectTemperature` | non-contact temperature of the object in view, °C |

The UDP discovery reply carries the same two values as `temp`/`objtemp`.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

### Wiring (I2C, address 0x5B)

| Pi Pin | Sensor Pin |
|--------|------------|
| 3.3V (Pin 1) | VCC |
| GND (Pin 6) | GND |
| GPIO 2 (Pin 3) | SDA |
| GPIO 3 (Pin 5) | SCL |

Enable I2C: `sudo raspi-config` → Interface Options → I2C.

## Setup

Install Go as described in [`../bme680/README.md`](../bme680/README.md)
(official tarball; the `apt` version is usually too old), then build —
all Go nodes live in one module, so build from this folder:

```bash
go build -o sensor_node_mlx90615 .
cp config.example.json config.json    # edit: api_key, i2c_bus
```

Or cross-compile from any machine and copy only the binary:

```bash
GOOS=linux GOARCH=arm64 go build -o sensor_node_mlx90615 .   # Pi 3/4/5 (64-bit OS)
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
./sensor_node_mlx90615
```

> Only one Sensor Playground node can run per board at a time — all
> nodes share ports 9132/9133.

## Testing

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

```json
{"sensor":"MLX90615","host":"raspberrypi","temperature":22.4,"objectTemperature":28.9}
```

`go test` checks the raw-word → °C conversion, including the error flag in
bit 15.

## Running as a Service

Use the systemd template from [`../bme680/README.md`](../bme680/README.md)
with the unit name `sensor-playground-mlx90615-go.service`.
