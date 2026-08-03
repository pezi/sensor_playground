"""
Sensor Tester Sensor Node — TCS34725 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a TCS34725 I2C sensor (RGB color, color temperature, illuminance).

- HTTPS REST API on port 9132 + UDP discovery on port 9133 (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import colorsys
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


class TCS34725Sensor:
    """Reads RGB color, color temperature, and lux from a TCS34725."""

    def __init__(self, i2c_bus=1):
        # Adafruit Blinka's `board` module has no pin map for many SBCs
        # (e.g. the NanoPi NEO2), so open the Linux I2C bus by number via
        # adafruit-extended-bus, honoring the configured i2c_bus.
        import adafruit_tcs34725
        from adafruit_extended_bus import ExtendedI2C

        self.sensor = adafruit_tcs34725.TCS34725(ExtendedI2C(i2c_bus))
        # The driver's defaults (2.4 ms integration, 1x gain) collect almost
        # no light: every channel reads 0 in normal room light and the
        # readings collapse to the same constants (lux 0, ~1391 K). Use the
        # same 154 ms / 4x the ESP32 sketch does, and leave the ADC enabled
        # instead of letting each read toggle it.
        self.sensor.integration_time = 154
        self.sensor.gain = 4
        self.sensor.active = True
        self.name = "TCS34725"

    def read(self):
        """Return full-key readings for the REST API."""
        red, green, blue = self.sensor.color_rgb_bytes
        color_temp = self.sensor.color_temperature
        lux = self.sensor.lux
        if color_temp is None or lux is None:
            return None
        return {
            "colorTemperature": round(color_temp),
            "lux": round(lux),
            "red": red,
            "green": green,
            "blue": blue,
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        return {"ct": full["colorTemperature"], "lux": full["lux"]}


class EmulatedTCS34725Sensor(TCS34725Sensor):
    """Generates plausible TCS34725 readings without hardware.

    A colored light slowly cycling through the hue circle every 30 seconds,
    with the color temperature swinging between warm and cool white and the
    illuminance drifting around a few hundred lux.
    """

    def __init__(self):
        self.name = "TCS34725"

    def read(self):
        t = time.time()
        r, g, b = colorsys.hsv_to_rgb((t / 30.0) % 1.0, 0.6, 0.9)
        return {
            "colorTemperature": round(4600 + 1900 * math.sin(t / 45.0)),
            "lux": round(300 + 200 * math.sin(t / 75.0) + random.uniform(-5, 5)),
            "red": round(r * 255),
            "green": round(g * 255),
            "blue": round(b * 255),
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
        print("Emulation mode: generating TCS34725 readings without hardware")
        _sensor = EmulatedTCS34725Sensor()
    else:
        print(f"Initializing TCS34725 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = TCS34725Sensor(i2c_bus=i2c_bus)

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
