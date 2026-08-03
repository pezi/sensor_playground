"""
Sensor Tester Sensor Node — SCD30 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a SCD30 I2C sensor (temperature, humidity, CO2).

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


class SCD30Sensor:
    """Reads temperature, humidity, and CO2 from an SCD30."""

    def __init__(self, i2c_bus=1):
        import inspect

        import smbus2
        from scd30_i2c import SCD30

        if "bus" in inspect.signature(SCD30.__init__).parameters:
            # scd30-i2c with a selectable bus (newer than 0.0.6).
            self.sensor = SCD30(bus=i2c_bus)
        else:
            # scd30-i2c 0.0.6 hardcodes smbus2.SMBus(1) in its constructor.
            # Build the driver without opening bus 1 (which may not exist on
            # this board) and point it at the configured bus instead.
            self.sensor = SCD30.__new__(SCD30)
            self.sensor._i2c_addr = 0x61
            self.sensor._i2c = smbus2.SMBus(i2c_bus)
        self.sensor.set_measurement_interval(2)
        self.sensor.start_periodic_measurement()
        self.name = "SCD30"

    def read(self):
        """Return full-key readings for the REST API."""
        if not self.sensor.get_data_ready():
            return None
        m = self.sensor.read_measurement()
        if m is None:
            return None
        # read_measurement() returns (co2, temperature, humidity).
        return {
            "temperature": round(m[1], 1),
            "humidity": round(m[2], 1),
            "co2": round(m[0], 1),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        return {
            "temp": full["temperature"],
            "hum": full["humidity"],
            "co2": full["co2"],
        }


class EmulatedSCD30Sensor(SCD30Sensor):
    """Generates plausible SCD30 readings without hardware.

    A quiet indoor room: temperature and humidity follow slow sines with
    different periods, the CO2 concentration does a bounded random walk
    between 400 and 1500 ppm.
    """

    def __init__(self):
        self.name = "SCD30"
        self._co2 = 600.0

    def read(self):
        t = time.time()
        self._co2 = min(max(self._co2 + random.uniform(-15, 15), 400.0), 1500.0)
        return {
            "temperature": round(
                22.0 + 2.0 * math.sin(t / 60.0) + random.uniform(-0.1, 0.1), 1
            ),
            "humidity": round(
                45.0 + 8.0 * math.sin(t / 97.0) + random.uniform(-0.5, 0.5), 1
            ),
            "co2": round(self._co2, 1),
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
        print("Emulation mode: generating SCD30 readings without hardware")
        _sensor = EmulatedSCD30Sensor()
    else:
        print(f"Initializing SCD30 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = SCD30Sensor(i2c_bus=i2c_bus)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_poll(
            _sensor.name,
            _api_key,
            lambda: {"sensor": _sensor.name, "host": _hostname, **(_sensor.read() or {})},
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
