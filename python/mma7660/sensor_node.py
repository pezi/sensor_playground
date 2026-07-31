"""
Sensor Tester Sensor Node — MMA7660 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a Grove 3-Axis Digital Accelerometer ±1.5g
(MMA7660FC). The raw axes are converted into roll / pitch angles plus the
total acceleration magnitude (g-force).

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


class MMA7660Sensor:
    """Reads roll/pitch/g-force from an MMA7660 (register access via smbus2).

    The MMA7660 reports each axis as a 6-bit two's-complement value with
    21.33 counts per g (±1.5 g range). Bit 6 of a sample is the alert flag,
    meaning the register was updated mid-read and must be read again.
    """

    _REG_X = 0x00
    _REG_SR = 0x08
    _REG_MODE = 0x07
    _COUNTS_PER_G = 21.33

    def __init__(self, i2c_bus=1, address=0x4C):
        from smbus2 import SMBus

        self.bus = SMBus(i2c_bus)
        self.address = address
        # Standby to configure, 32 samples/s, then active mode.
        self.bus.write_byte_data(self.address, self._REG_MODE, 0x00)
        self.bus.write_byte_data(self.address, self._REG_SR, 0x02)
        self.bus.write_byte_data(self.address, self._REG_MODE, 0x01)
        self.name = "MMA7660"

    def _read_axes(self):
        """Return (x, y, z) in g, re-reading while the alert bit is set."""
        for _ in range(10):
            raw = self.bus.read_i2c_block_data(self.address, self._REG_X, 3)
            if any(value & 0x40 for value in raw):
                continue
            axes = [v - 64 if v > 31 else v for v in raw]
            return tuple(v / self._COUNTS_PER_G for v in axes)
        raise OSError("MMA7660 kept reporting the alert bit")

    def read(self):
        """Return full-key readings for the REST API."""
        x, y, z = self._read_axes()
        roll = math.degrees(math.atan2(y, z))
        pitch = math.degrees(math.atan2(-x, math.sqrt(y * y + z * z)))
        gforce = math.sqrt(x * x + y * y + z * z)
        return {
            "roll": round(roll, 1),
            "pitch": round(pitch, 1),
            "gforce": round(gforce, 2),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        return self.read()


class EmulatedMMA7660Sensor(MMA7660Sensor):
    """Generates plausible MMA7660 readings without hardware.

    A gently rocking, near-level board: roll and pitch follow slow sines
    with different periods and the g-force stays around 1 g.
    """

    def __init__(self):
        self.name = "MMA7660"

    def read(self):
        t = time.time()
        return {
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
        print("Emulation mode: generating MMA7660 readings without hardware")
        _sensor = EmulatedMMA7660Sensor()
    else:
        print(f"Initializing MMA7660 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = MMA7660Sensor(i2c_bus=i2c_bus)

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
