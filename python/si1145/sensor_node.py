"""
Sensor Tester Sensor Node — SI1145 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a SI1145 I2C sensor (visible light, infrared, UV index).

- HTTPS REST API on port 9132 + UDP discovery on port 9133 (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import math
import random
import socket
import sys
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Sensor ------------------------------------------------------------------


class SI1145Sensor:
    """Reads visible light, IR, and UV index from an SI1145."""

    def __init__(self, i2c_bus=1):
        # The SI1145 library evaluates `busnum=I2C.get_default_bus()` as a
        # default argument at import time, and Adafruit-GPIO cannot detect
        # the I2C bus on many SBCs (e.g. the NanoPi NEO2), so the import
        # itself raises "Could not determine default I2C bus for platform".
        # Patch the lookup to the configured bus before importing the driver.
        import Adafruit_GPIO.I2C as adafruit_i2c

        adafruit_i2c.get_default_bus = lambda: i2c_bus
        import SI1145.SI1145 as si1145_driver

        self.sensor = si1145_driver.SI1145(busnum=i2c_bus)
        self.name = "SI1145"

    def read(self):
        """Return full-key readings for the REST API."""
        return {
            "visible": int(self.sensor.readVisible()),
            "ir": int(self.sensor.readIR()),
            # The chip reports the UV index multiplied by 100.
            "uv": round(self.sensor.readUV() / 100.0, 1),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        return {"vis": full["visible"], "ir": full["ir"], "uv": full["uv"]}


class EmulatedSI1145Sensor(SI1145Sensor):
    """Generates plausible SI1145 readings without hardware.

    Indoor light near a window: visible and IR counts hover around the
    chip's dark baseline (~260) with slow sines and a little flicker,
    while the UV index sweeps between 0 and 8 over ten minutes.
    """

    def __init__(self):
        self.name = "SI1145"

    def read(self):
        t = time.time()
        return {
            "visible": round(262 + 40 * math.sin(t / 90.0) + random.uniform(-3, 3)),
            "ir": round(253 + 30 * math.sin(t / 70.0) + random.uniform(-3, 3)),
            "uv": round(4.0 + 4.0 * math.sin(t / 600.0), 1),
        }

_sensor = None
_api_key = ""
_hostname = ""


# -- Main --------------------------------------------------------------------


def main():
    global _sensor, _api_key, _hostname

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    _hostname = config.get("hostname", "") or socket.gethostname()
    i2c_bus = config.get("i2c_bus", 1)
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating SI1145 readings without hardware")
        _sensor = EmulatedSI1145Sensor()
    else:
        print(f"Initializing SI1145 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = SI1145Sensor(i2c_bus=i2c_bus)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_poll(
            _sensor.name,
            _api_key,
            lambda: {"sensor": _sensor.name, "host": _hostname, **_sensor.read()},
        )
        return

    wifi.start_discovery_thread(
        _sensor.name, _hostname, wifi.HTTPS_PORT, _sensor.read_discovery
    )
    wifi.run_rest_server(
        _sensor.name, _api_key, _hostname, _sensor.read, ssl_cert, ssl_key
    )


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
