"""
Sensor Tester Sensor Node — BME280 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a BME280 I2C sensor (temperature, humidity, pressure).

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


class BME280Sensor:
    """Reads temperature, humidity, and pressure from a BME280."""

    def __init__(self, i2c_bus=1, address=0x76):
        import bme280
        from smbus2 import SMBus

        self._driver = bme280
        self.bus = SMBus(i2c_bus)
        self.address = address
        self.calibration = bme280.load_calibration_params(self.bus, address)
        self.name = "BME280"

    def read(self):
        """Return full-key readings for the REST API."""
        sample = self._driver.sample(self.bus, self.address, self.calibration)
        return {
            "temperature": round(sample.temperature, 1),
            "humidity": round(sample.humidity, 1),
            "pressure": round(sample.pressure, 2),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        return {
            "temp": full["temperature"],
            "hum": full["humidity"],
            "press": full["pressure"],
        }


class EmulatedBME280Sensor(BME280Sensor):
    """Generates plausible BME280 readings without hardware.

    A comfortable indoor climate: temperature, humidity and pressure drift
    on slow sines with different periods around 22 °C / 45 %RH / 1013 hPa,
    plus a little measurement noise.
    """

    def __init__(self):
        self.name = "BME280"

    def read(self):
        t = time.time()
        return {
            "temperature": round(
                22.0 + 2.0 * math.sin(t / 60.0) + random.uniform(-0.1, 0.1), 1
            ),
            "humidity": round(
                45.0 + 8.0 * math.sin(t / 97.0) + random.uniform(-0.5, 0.5), 1
            ),
            "pressure": round(
                1013.0 + 3.0 * math.sin(t / 300.0) + random.uniform(-0.2, 0.2), 2
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
        print("Emulation mode: generating BME280 readings without hardware")
        _sensor = EmulatedBME280Sensor()
    else:
        print(f"Initializing BME280 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = BME280Sensor(i2c_bus=i2c_bus)

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
