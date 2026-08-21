# BME680 Sensor Node for Sensor Playground (Rust)

This Rust program implements the Sensor Playground sensor interface on
single-board computers with a BME680 I2C sensor (temperature, humidity,
pressure, IAQ). Over Wi-Fi the Sensor Playground app discovers this node
via UDP broadcast (port 9133) and polls it for data over HTTPS (port
9132, `X-Api-Key` header); over BLE the node advertises the Sensor
Playground GATT service instead.

It is the Rust counterpart of `../../python/bme680/` and — like the
Python node — supports both transports (Wi-Fi and BLE). Of the three
language ports (Go, Node.js, Rust), this is the only one with BLE.

The sensor uses a compact self-contained driver (`src/driver.rs`) with
the same integer compensation formulas as the Go and Node.js ports and
Bosch-datasheet fixes for signed gas calibration and sample validity.
The HTTPS server is a minimal hand-rolled rustls server — the node serves
a single `GET /` route, which does not justify an async framework's
dependency tree.

## Supported Platforms

| Board | I2C Bus (`i2c_bus` in config) |
|-------|-------------------------------|
| Raspberry Pi | `1` (default) |
| NanoPi (Armbian) | `0` |
| Banana Pi (Armbian) | `2` |

### Wiring (I2C)

| Pi Pin     | Sensor Pin |
|------------|------------|
| 3.3V (Pin 1) | VCC     |
| GND (Pin 6)  | GND     |
| GPIO 2 (Pin 3) | SDA   |
| GPIO 3 (Pin 5) | SCL   |

Enable I2C on the Pi:

```bash
sudo raspi-config   # Interface Options > I2C > Enable
```

## Software Setup

Install Rust and the build prerequisites (the D-Bus headers are needed
by the BLE support):

```bash
sudo apt install -y build-essential pkg-config libdbus-1-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source "$HOME/.cargo/env"
```

All Rust nodes form one Cargo workspace with the shared
[`../common/`](../common) crate. On a non-Pi development host, build
from this folder normally:

```bash
cargo build --release
cp config.example.json config.json
# Edit config.json: set api_key and the i2c_bus for your board
```

### Native Raspberry Pi builds

On a 64-bit Raspberry Pi 3 Model B+ running Debian 13, `rustc 1.97.1`
reproducibly crashed with `SIGSEGV` in `librustc_driver` while compiling
small dependencies. Increasing `RUST_MIN_STACK` did not fix it. The
same workspace completed a release build with `rustc 1.96.0`.

For a native build on a Raspberry Pi, use the known-working compiler
and limit Cargo to one job:

```bash
rustup toolchain install 1.96.0 --profile minimal
rustup override set 1.96.0
cargo build --release -j 1
```

The override is scoped to this project directory. After a newer Rust
release fixes the crash, remove it with `rustup override unset` and
retry the current stable toolchain.

Also check the Pi while the build is running:

```bash
vcgencmd get_throttled
vcgencmd measure_temp
```

For example, `throttled=0xd0000` records that undervoltage, throttling,
and the soft temperature limit have occurred since boot, even when no
condition is currently active. Use a suitable 5 V power supply and
adequate cooling before trusting long native builds.

**Prefer cross-compiling for a Raspberry Pi.** A release build on a
small Pi takes 10–30 minutes and can fail outright on a marginal power
supply (rustc crashes under sustained load). From another machine:

```bash
rustup target add aarch64-unknown-linux-gnu
brew install zig cargo-zigbuild          # or pip install cargo-zigbuild
cargo zigbuild --release --target aarch64-unknown-linux-gnu
scp ../target/aarch64-unknown-linux-gnu/release/sensor_node_bme680 pi:
```

(bluer's libdbus dependency is built from vendored sources, so no target
sysroot is needed.)

## Transports

The transport is selected via `"transport"` in `config.json`:

- `"wifi"` (default) — HTTPS REST server on port 9132 plus UDP discovery
  on port 9133. Requires the SSL certificates below.
- `"ble"` — BLE GATT server, identical protocol to the ESP32 sketches
  and the Python node (service `d1a51b00-0001-…`, auth by writing the
  API key, framed 1 Hz notifications). No SSL certificates and no UDP
  discovery; BLE advertising is the discovery. Only one BLE node can run
  per board, and the node must run as **root**: on Raspberry Pi,
  bluetoothd frequently accepts the advertisement registration without
  the controller ever transmitting (and on some kernels rejects it
  outright), so after every registration the node verifies the kernel's
  advertising state and, if the ad is not on air, drives the kernel's
  Bluetooth management socket directly — which needs `CAP_NET_ADMIN`.
  This is the same workaround the Python node uses. Note that without
  extended-advertising support (e.g. the Pi 3's controller) the kernel
  falls back to a 1.28 s advertising interval, so scanners can need
  ~10 s to discover the node.

## Emulation

Set `"emulation": true` in `config.json` to run the node without the
sensor hardware — it then serves plausible generated readings (works
with both transports; on macOS/Windows only Wi-Fi + emulation builds).

## SSL Certificates (Wi-Fi transport only)

Generate a self-signed certificate for HTTPS:

```bash
openssl req -x509 -nodes -days 3650 -newkey rsa:2048 \
  -keyout key.pem -out cert.pem -subj "/CN=SensorPlayground"
```

The generated `cert.pem` and `key.pem` are referenced in `config.json`
and excluded from Git. They are resolved relative to the working
directory; `config.json` is looked up in the working directory first,
then next to the binary.

## Usage

```bash
../target/release/sensor_node_bme680        # Wi-Fi
sudo ../target/release/sensor_node_bme680   # BLE (needs root, see above)
```

> Only one Sensor Playground node can run per board at a time — the
> Python, Go, Node.js and Rust nodes all share ports 9132/9133 (and the
> BLE advertising instance).

## Testing

Run the compensation and IAQ state regression tests:

```bash
cargo test
```

The compensation suite shares the 200 corrected golden vectors used by
the Go and Node.js implementations.

Wi-Fi:

```bash
curl -k -H "X-Api-Key: your-sensor-api-key" https://localhost:9132/
```

BLE: from another machine, run the Python BLE test client
(`../../python/common/ble_client_test.py`) or use nRF Connect — write
the API key to the auth characteristic (`…0003…`), then subscribe to the
data characteristic (`…0002…`).

## Running as a Service (optional)

Create `/etc/systemd/system/sensor-playground-bme680-rust.service`:

```ini
[Unit]
Description=Sensor Playground BME680 Node (Rust)
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=pi
WorkingDirectory=/home/pi/sensor-bme680-rust
ExecStart=/home/pi/sensor-bme680-rust/target/release/sensor_node_bme680
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

Then enable and start:

```bash
sudo systemctl daemon-reload
sudo systemctl enable sensor-playground-bme680-rust
sudo systemctl start sensor-playground-bme680-rust
```

For the BLE transport, replace the `[Unit]` dependencies with
`After=bluetooth.target` / `Wants=bluetooth.target` and set `User=root`
(required for the kernel advertising workaround).

## IAQ Calculation

The BME680 IAQ score is computed using a rolling-baseline algorithm
(ported from [dart_periphery](https://pub.dev/packages/dart_periphery)).
It maintains a window of 50 gas resistance readings to establish a
baseline, then scores gas (75%) and humidity (25%) relative to their
baselines. The score stabilizes after approximately 50 readings.
