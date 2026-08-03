"""
Sensor Tester Sensor Node — MLX90615 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a Grove Digital Infrared Temperature Sensor
(MLX90615): the non-contact object temperature plus the sensor's own
ambient temperature.

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


class MLX90615Sensor:
    """Reads object and ambient temperature from an MLX90615.

    Register access via smbus2: SMBus word reads from RAM register 0x26
    (ambient) and 0x27 (object); temperature = raw * 0.02 K - 273.15.
    """

    _REG_AMBIENT = 0x26
    _REG_OBJECT = 0x27

    def __init__(self, i2c_bus=1, address=0x5B):
        from smbus2 import SMBus

        self.bus = SMBus(i2c_bus)
        self.address = address
        self.name = "MLX90615"

    def _read_temperature(self, register):
        """Return one temperature in °C, or None on an error flag."""
        raw = self.bus.read_word_data(self.address, register)
        if raw & 0x8000:
            return None
        return raw * 0.02 - 273.15

    def read(self):
        """Return full-key readings for the REST API."""
        ambient = self._read_temperature(self._REG_AMBIENT)
        obj = self._read_temperature(self._REG_OBJECT)
        if ambient is None or obj is None:
            return None
        return {
            "temperature": round(ambient, 1),
            "objectTemperature": round(obj, 1),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        return {
            "temp": full["temperature"],
            "objtemp": full["objectTemperature"],
        }


class EmulatedMLX90615Sensor(MLX90615Sensor):
    """Generates plausible MLX90615 readings without hardware.

    An ambient temperature drifting on a slow sine around 22 °C and a
    warmer object (around 28 °C) in the field of view following its own
    slower sine, both with a little measurement noise.
    """

    def __init__(self):
        self.name = "MLX90615"

    def read(self):
        t = time.time()
        return {
            "temperature": round(
                22.0 + 2.0 * math.sin(t / 60.0) + random.uniform(-0.1, 0.1), 1
            ),
            "objectTemperature": round(
                28.0 + 3.0 * math.sin(t / 45.0) + random.uniform(-0.2, 0.2), 1
            ),
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
        print("Emulation mode: generating MLX90615 readings without hardware")
        _sensor = EmulatedMLX90615Sensor()
    else:
        print(f"Initializing MLX90615 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = MLX90615Sensor(i2c_bus=i2c_bus)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_poll(
            _sensor.name,
            _api_key,
            lambda: {
                "sensor": _sensor.name,
                "host": _hostname,
                **(_sensor.read() or {}),
            },
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
