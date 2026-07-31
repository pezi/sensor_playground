"""
Sensor Tester Sensor Node — MCP9808 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a MCP9808 I2C sensor (temperature).

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


class MCP9808Sensor:
    """Reads temperature from an MCP9808 (register access via smbus2)."""

    _REG_AMBIENT_TEMP = 0x05

    def __init__(self, i2c_bus=1, address=0x18):
        from smbus2 import SMBus

        self.bus = SMBus(i2c_bus)
        self.address = address
        self.name = "MCP9808"

    def read(self):
        """Return full-key readings for the REST API."""
        raw = self.bus.read_i2c_block_data(
            self.address, self._REG_AMBIENT_TEMP, 2
        )
        value = (raw[0] << 8) | raw[1]
        temperature = (value & 0x0FFF) / 16.0
        if value & 0x1000:
            temperature -= 256.0
        return {"temperature": round(temperature, 2)}

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        return {"temp": full["temperature"]}


class EmulatedMCP9808Sensor(MCP9808Sensor):
    """Generates plausible MCP9808 readings without hardware.

    A room temperature drifting on a slow sine around 22 °C, plus a little
    measurement noise.
    """

    def __init__(self):
        self.name = "MCP9808"

    def read(self):
        t = time.time()
        return {
            "temperature": round(
                22.0 + 2.0 * math.sin(t / 60.0) + random.uniform(-0.1, 0.1), 2
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
        print("Emulation mode: generating MCP9808 readings without hardware")
        _sensor = EmulatedMCP9808Sensor()
    else:
        print(f"Initializing MCP9808 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = MCP9808Sensor(i2c_bus=i2c_bus)

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
