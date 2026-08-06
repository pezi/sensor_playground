"""
Sensor Tester Sensor Node — VL53L0X (Python)

Implements the *push* variant of the Sensor Tester Sensor Interface on
single-board computers (Raspberry Pi & co.) with a VL53L0X time-of-flight
distance sensor. The node measures continuously and pushes one JSON message
({"distance": <mm>}, null when out of range) whenever the distance changes,
or at least once per second as a heartbeat.

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import random
import socket
import sys
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Publish policy -----------------------------------------------------------

MEASURE_INTERVAL = 0.1
HEARTBEAT = 1.0
MIN_DELTA_MM = 3

# -- Sensor ------------------------------------------------------------------


class VL53L0XSensor:
    """Reads distance in millimeters from a VL53L0X."""

    def __init__(self, i2c_bus=1):
        # Adafruit Blinka's `board` module has no pin map for many SBCs
        # (e.g. the NanoPi NEO2), so open the Linux I2C bus by number via
        # adafruit-extended-bus, honoring the configured i2c_bus.
        import adafruit_vl53l0x
        from adafruit_extended_bus import ExtendedI2C

        self.sensor = adafruit_vl53l0x.VL53L0X(ExtendedI2C(i2c_bus))
        self.name = "VL53L0X"

    def read_mm(self):
        """Return the distance in mm, or None when out of range."""
        mm = self.sensor.range
        # The VL53L0X reports ~8190 mm when no target is in range.
        return mm if 0 < mm < 8000 else None


class EmulatedVL53L0XSensor(VL53L0XSensor):
    """Generates plausible VL53L0X readings without hardware.

    A target sweeping back and forth between 100 and 1200 mm (20 s period),
    occasionally leaving the measuring range.
    """

    def __init__(self):
        self.name = "VL53L0X"

    def read_mm(self):
        if random.random() < 0.02:
            return None
        phase = (time.time() % 20.0) / 20.0
        return round(100 + 1100 * (1 - abs(2 * phase - 1)))


# -- WebSocket push server -----------------------------------------------------

_api_key = ""


async def measure_loop(sensor, publish):
    """Measure continuously; publish on change, range flip, or heartbeat."""
    loop = asyncio.get_event_loop()
    last_sent = None
    last_publish = 0.0
    ever_published = False

    while True:
        mm = sensor.read_mm()
        now = loop.time()
        changed = (
            not ever_published
            or (mm is None) != (last_sent is None)
            or (
                mm is not None
                and last_sent is not None
                and abs(mm - last_sent) >= MIN_DELTA_MM
            )
        )
        if changed or now - last_publish >= HEARTBEAT:
            await publish({"distance": mm})
            ever_published = True
            last_sent = mm
            last_publish = now
        await asyncio.sleep(MEASURE_INTERVAL)


async def main_async(sensor):
    server = wifi.WsPushServer(_api_key)
    async with server.serve():
        await measure_loop(sensor, server.broadcast)


async def main_ble(sensor, api_key):
    """Push readings over BLE instead of WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(sensor.name, api_key)
    await transport.start()
    try:
        await measure_loop(sensor, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    i2c_bus = config.get("i2c_bus", 1)

    if config.get("emulation", False):
        print("Emulation mode: generating VL53L0X readings without hardware")
        sensor = EmulatedVL53L0XSensor()
    else:
        print("Initializing VL53L0X sensor...")
        sensor = VL53L0XSensor(i2c_bus=i2c_bus)

    if config.get("transport", "wifi") == "ble":
        asyncio.run(main_ble(sensor, _api_key))
        return

    wifi.start_discovery_thread(sensor.name, hostname, wifi.WS_PORT)

    asyncio.run(main_async(sensor))


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
