"""
Sensor Tester Sensor Node — SHT31 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a SHT31 I2C sensor (temperature, humidity).

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


class SHT31Sensor:
    """Reads temperature and humidity from an SHT31 (single-shot mode)."""

    def __init__(self, i2c_bus=1, address=0x44):
        from smbus2 import SMBus, i2c_msg

        self.bus = SMBus(i2c_bus)
        self.address = address
        self._i2c_msg = i2c_msg
        self.name = "SHT31"

    def read(self):
        """Return full-key readings for the REST API."""
        import time

        # Single-shot measurement, high repeatability, no clock stretching.
        self.bus.i2c_rdwr(self._i2c_msg.write(self.address, [0x24, 0x00]))
        time.sleep(0.02)
        msg = self._i2c_msg.read(self.address, 6)
        self.bus.i2c_rdwr(msg)
        raw = list(msg)
        temp_raw = (raw[0] << 8) | raw[1]
        hum_raw = (raw[3] << 8) | raw[4]
        return {
            "temperature": round(-45.0 + 175.0 * temp_raw / 65535.0, 1),
            "humidity": round(100.0 * hum_raw / 65535.0, 1),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        return {"temp": full["temperature"], "hum": full["humidity"]}


class EmulatedSHT31Sensor(SHT31Sensor):
    """Generates plausible SHT31 readings without hardware.

    A comfortable indoor climate: temperature and humidity drift on slow
    sines with different periods around 22 °C / 45 %RH, plus a little
    measurement noise.
    """

    def __init__(self):
        self.name = "SHT31"

    def read(self):
        t = time.time()
        return {
            "temperature": round(
                22.0 + 2.0 * math.sin(t / 60.0) + random.uniform(-0.1, 0.1), 1
            ),
            "humidity": round(
                45.0 + 8.0 * math.sin(t / 97.0) + random.uniform(-0.5, 0.5), 1
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
        print("Emulation mode: generating SHT31 readings without hardware")
        _sensor = EmulatedSHT31Sensor()
    else:
        print(f"Initializing SHT31 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = SHT31Sensor(i2c_bus=i2c_bus)

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
