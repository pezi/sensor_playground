"""
Sensor Tester Sensor Node — SGP30 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a SGP30 I2C sensor (eCO2, TVOC).

- HTTPS REST API on port 9132 + UDP discovery on port 9133 (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import random
import socket
import sys
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Sensor ------------------------------------------------------------------


class SGP30Sensor:
    """Reads eCO2 and TVOC from an SGP30."""

    def __init__(self, i2c_bus=1):
        from sgp30 import SGP30
        from smbus2 import SMBus, i2c_msg

        # The library needs both the bus and the i2c_msg factory; passing only
        # i2c_dev leaves _i2c_msg as None (omitting both would hardcode bus 1).
        self.sensor = SGP30(i2c_dev=SMBus(i2c_bus), i2c_msg=i2c_msg)
        print("Warming up SGP30 (about 15 seconds)...")
        self.sensor.start_measurement()
        self.name = "SGP30"

    def read(self):
        """Return full-key readings for the REST API."""
        result = self.sensor.get_air_quality()
        return {
            "eco2": result.equivalent_co2,
            "tvoc": result.total_voc,
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        return self.read()


class EmulatedSGP30Sensor(SGP30Sensor):
    """Generates plausible SGP30 readings without hardware.

    Indoor air quality drifting around typical office values: eCO2 does a
    bounded random walk between 400 and 1500 ppm, TVOC between 0 and
    600 ppb.
    """

    def __init__(self):
        self.name = "SGP30"
        self._eco2 = 600.0
        self._tvoc = 60.0

    def read(self):
        self._eco2 = min(max(self._eco2 + random.uniform(-15, 15), 400.0), 1500.0)
        self._tvoc = min(max(self._tvoc + random.uniform(-8, 8), 0.0), 600.0)
        return {
            "eco2": round(self._eco2),
            "tvoc": round(self._tvoc),
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
        print("Emulation mode: generating SGP30 readings without hardware")
        _sensor = EmulatedSGP30Sensor()
    else:
        print(f"Initializing SGP30 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = SGP30Sensor(i2c_bus=i2c_bus)

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
    main()
