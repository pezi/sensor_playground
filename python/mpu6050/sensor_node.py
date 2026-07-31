"""
Sensor Tester Sensor Node — MPU6050 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with an MPU6050 6-axis IMU. The accelerometer axes are
converted into roll / pitch angles plus the total acceleration magnitude
(g-force); the on-die temperature sensor is reported as well.

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default) — the app streams accelerometers rather than polling them, or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import math
import random
import socket
import sys
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# The app streams accelerometers instead of polling them, so readings are
# pushed at the same 250 ms cadence the BLE transport and the ESP32 use.
PUSH_INTERVAL = 0.25

# -- Sensor ------------------------------------------------------------------


class MPU6050Sensor:
    """Reads roll/pitch/g-force and die temperature from an MPU6050.

    Register access via smbus2: the chip wakes from sleep by clearing
    PWR_MGMT_1, then a 14-byte burst read from ACCEL_XOUT_H delivers the
    accelerometer (16384 LSB/g at the default ±2 g range), temperature
    (raw/340 + 36.53 °C) and gyroscope values.
    """

    _REG_PWR_MGMT_1 = 0x6B
    _REG_ACCEL_XOUT_H = 0x3B
    _LSB_PER_G = 16384.0

    def __init__(self, i2c_bus=1, address=0x68):
        from smbus2 import SMBus

        self.bus = SMBus(i2c_bus)
        self.address = address
        # Wake the chip (it powers up in sleep mode).
        self.bus.write_byte_data(self.address, self._REG_PWR_MGMT_1, 0x00)
        self.name = "MPU6050"

    def read(self):
        """Return full-key readings for the REST API."""
        raw = self.bus.read_i2c_block_data(
            self.address, self._REG_ACCEL_XOUT_H, 14
        )

        def s16(index):
            value = (raw[index] << 8) | raw[index + 1]
            return value - 65536 if value > 32767 else value

        ax = s16(0) / self._LSB_PER_G
        ay = s16(2) / self._LSB_PER_G
        az = s16(4) / self._LSB_PER_G
        temperature = s16(6) / 340.0 + 36.53

        roll = math.degrees(math.atan2(ay, az))
        pitch = math.degrees(math.atan2(-ax, math.sqrt(ay * ay + az * az)))
        gforce = math.sqrt(ax * ax + ay * ay + az * az)

        return {
            "temperature": round(temperature, 1),
            "roll": round(roll, 1),
            "pitch": round(pitch, 1),
            "gforce": round(gforce, 2),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        return {
            "temp": full["temperature"],
            "roll": full["roll"],
            "pitch": full["pitch"],
            "gforce": full["gforce"],
        }


class EmulatedMPU6050Sensor(MPU6050Sensor):
    """Generates plausible MPU6050 readings without hardware.

    A gently rocking, near-level board: roll and pitch follow slow sines
    with different periods, the g-force stays around 1 g and the die
    temperature drifts a few degrees above room temperature.
    """

    def __init__(self):
        self.name = "MPU6050"

    def read(self):
        t = time.time()
        return {
            "temperature": round(36.0 + 2.0 * math.sin(t / 60.0), 1),
            "roll": round(8.0 * math.sin(t / 7.0) + random.uniform(-0.3, 0.3), 1),
            "pitch": round(5.0 * math.sin(t / 11.0) + random.uniform(-0.3, 0.3), 1),
            "gforce": round(1.0 + random.uniform(-0.02, 0.02), 2),
        }


# -- WebSocket push server ---------------------------------------------------

_sensor = None
_api_key = ""
_hostname = ""


async def push_loop(publish):
    """Push a reading every PUSH_INTERVAL, like the ESP32 sketch."""
    while True:
        data = _sensor.read()
        if data:
            await publish({"sensor": _sensor.name, "host": _hostname, **data})
        await asyncio.sleep(PUSH_INTERVAL)


async def main_async():
    server = wifi.WsPushServer(_api_key)
    async with server.serve():
        await push_loop(server.broadcast)


# -- Main --------------------------------------------------------------------


def main():
    global _sensor, _api_key, _hostname

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    _hostname = config.get("hostname", "") or socket.gethostname()
    i2c_bus = config.get("i2c_bus", 1)

    if config.get("emulation", False):
        print("Emulation mode: generating MPU6050 readings without hardware")
        _sensor = EmulatedMPU6050Sensor()
    else:
        print(f"Initializing MPU6050 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = MPU6050Sensor(i2c_bus=i2c_bus)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_poll(
            _sensor.name,
            _api_key,
            lambda: {"sensor": _sensor.name, "host": _hostname, **_sensor.read()},
            interval=ble_transport.MOTION_NOTIFY_INTERVAL,
        )
        return

    wifi.start_discovery_thread(
        _sensor.name, _hostname, wifi.WS_PORT, _sensor.read_discovery
    )

    asyncio.run(main_async())


if __name__ == "__main__":
    main()
