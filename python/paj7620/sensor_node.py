"""
Sensor Tester Sensor Node — PAJ7620 Grove Gesture (Python)

Implements the *push* variant of the Sensor Tester Sensor Interface on
single-board computers (Raspberry Pi & co.) with a Grove Gesture sensor
(PAJ7620U2). The node polls the sensor and pushes one JSON message
({"gesture": "forward"}) per detected gesture.

The gesture strings match the app's Gesture enum names. The grove.py
driver reports the nine basic gestures; the combined gestures
(forwardBackward, rightLeft, ...) of the ESP32 sketch are not detected.

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

from unittest.mock import MagicMock

# Inject a dummy RPi.GPIO module to bypass the Grove library dependency for non RPi Socs
sys.modules['RPi'] = MagicMock()
sys.modules['RPi.GPIO'] = MagicMock()

# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

POLL_INTERVAL = 0.1

# -- Sensor ------------------------------------------------------------------


class PAJ7620Sensor:
    """Reads gestures from a PAJ7620U2 via the grove.py driver."""

    # grove.py return_gesture() codes mapped to the app's Gesture enum names.
    GESTURES = {
        1: "forward",
        2: "backward",
        3: "right",
        4: "left",
        5: "up",
        6: "down",
        7: "clockwise",
        8: "antiClockwise",
        9: "wave",
    }

    def __init__(self, i2c_bus=1):
        # grove.py wraps a process-wide singleton I2C bus that the gesture
        # driver creates at import time, defaulting to bus 1. Seed the
        # singleton on the configured bus first so the driver reuses it
        # instead of opening bus 1 (which fails on boards like the NanoPi
        # NEO2 where the sensor is on bus 0).
        from grove.i2c import Bus

        Bus(i2c_bus)
        from grove.grove_gesture_sensor import gesture

        self.sensor = gesture()
        self._init_sensor(i2c_bus)
        self.name = "PAJ7620"

    _WAKE_ATTEMPTS = 3
    _I2C_ADDRESS = 0x73

    def _init_sensor(self, i2c_bus):
        """Initialise the driver, waking the sensor if it does not answer.

        The PAJ7620 keeps its I2C interface powered down until bus activity
        wakes it, and does not acknowledge the transaction that does the
        waking. The driver's init() opens with a register write, so on an
        unpoked sensor that write fails with OSError(EIO) and the node exits
        before it can start. (The driver hints at this by selecting bank 0
        twice, but does not catch the first attempt failing.) Poke the bus,
        give the sensor a moment, and try again.
        """
        for attempt in range(self._WAKE_ATTEMPTS):
            try:
                self.sensor.init()
                return
            except OSError as exc:
                if attempt == self._WAKE_ATTEMPTS - 1:
                    raise RuntimeError(
                        f"PAJ7620 did not respond on i2c bus {i2c_bus} "
                        f"({exc}). Check the wiring and that the sensor "
                        f"appears at {self._I2C_ADDRESS:#04x}."
                    ) from exc
                self._poke(i2c_bus)

    @classmethod
    def _poke(cls, i2c_bus):
        """Send one throwaway transaction to wake the sensor."""
        from smbus2 import SMBus

        try:
            with SMBus(i2c_bus) as bus:
                bus.write_byte(cls._I2C_ADDRESS, 0x00)
        except OSError:
            pass  # the waking transaction is itself not acknowledged
        time.sleep(0.05)

    def read_gesture(self):
        """Return the detected gesture name, or None if nothing happened."""
        code = self.sensor.return_gesture()
        return self.GESTURES.get(code)


class EmulatedPAJ7620Sensor(PAJ7620Sensor):
    """Generates plausible PAJ7620 gestures without hardware.

    Reports one random gesture from the gesture map every three to five
    seconds; polls in between return no gesture.
    """

    def __init__(self):
        self.name = "PAJ7620"
        self._next_at = time.time() + random.uniform(3.0, 5.0)

    def read_gesture(self):
        """Return the detected gesture name, or None if nothing happened."""
        if time.time() < self._next_at:
            return None
        self._next_at = time.time() + random.uniform(3.0, 5.0)
        return random.choice(list(self.GESTURES.values()))


# -- WebSocket push server -----------------------------------------------------

_api_key = ""


async def gesture_loop(sensor, publish):
    """Poll the sensor and push each detected gesture."""
    while True:
        name = sensor.read_gesture()
        if name is not None:
            print(f"Gesture: {name}")
            await publish({"gesture": name})
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(sensor):
    server = wifi.WsPushServer(_api_key)
    async with server.serve():
        await gesture_loop(sensor, server.broadcast)


async def main_ble(sensor, api_key):
    """Push readings over BLE instead of WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(sensor.name, api_key)
    await transport.start()
    try:
        await gesture_loop(sensor, transport.publish)
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
        print("Emulation mode: generating PAJ7620 gestures without hardware")
        sensor = EmulatedPAJ7620Sensor()
    else:
        print("Initializing PAJ7620 gesture sensor...")
        sensor = PAJ7620Sensor(i2c_bus=i2c_bus)

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
