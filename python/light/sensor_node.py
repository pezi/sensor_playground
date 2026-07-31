"""
Sensor Tester Sensor Node — Grove Light Sensor (Python)

Implements the Sensor Tester Sensor Interface on single-board computers with a
Grove Light Sensor — an analog photo-resistor. It reports a raw brightness
value (higher = brighter) under the JSON key `light`.
https://wiki.seeedstudio.com/Grove-Light_Sensor/

The Raspberry Pi has no analog input, so an analog sensor needs an extension
hat with an ADC. This node reads it through the sibling `extension_hat`
helper — there is no direct-GPIO option (unlike the digital contact node):

    "hat_type": "grove"       Seeed Grove Base Hat (12-bit ADC, 0-4095)
    "hat_type": "nano"        FriendlyARM NanoHat Hub (analogRead)
    "hat_type": "grovePlus"   Seeed GrovePi+ (10-bit ADC)

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


# The extension_hat helper and the shared BLE transport live in sibling folders.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "extension_hat"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Sensor ------------------------------------------------------------------


class LightSensor:
    """Reads a Grove Light Sensor through an extension hat's ADC."""

    def __init__(self, hat_type="grove", pin=0, i2c_bus=1):
        from extension_hat import GroveBaseHat, GrovePiPlusHat, NanoHatHub

        if hat_type == "grove":
            self._hat = GroveBaseHat(i2c_bus)
            self._read = lambda: self._hat.read_adc_raw(pin)
        elif hat_type == "nano":
            self._hat = NanoHatHub(i2c_bus)
            self._read = lambda: self._hat.analog_read(pin)
        elif hat_type == "grovePlus":
            self._hat = GrovePiPlusHat(i2c_bus)
            self._read = lambda: self._hat.analog_read(pin)
        else:
            raise ValueError(f"Unknown hat_type {hat_type!r}")
        self.name = "LIGHT"

    def read(self):
        """Return full-key readings for the REST API."""
        return {"light": self._read()}

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        # The light value uses the same key on both payloads.
        return self.read()


class EmulatedLightSensor(LightSensor):
    """Generates plausible Grove Light Sensor readings without hardware.

    Daylight through a window: the raw brightness follows a slow sine
    around a few hundred counts with a little flicker, never below zero.
    """

    def __init__(self):
        self.name = "LIGHT"

    def read(self):
        t = time.time()
        raw = 400 + 350 * math.sin(t / 120.0) + random.uniform(-15, 15)
        return {"light": max(0, round(raw))}

_sensor = None
_api_key = ""
_hostname = ""


# -- Main --------------------------------------------------------------------


def main():
    global _sensor, _api_key, _hostname

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    _hostname = config.get("hostname", "") or socket.gethostname()
    hat_type = config.get("hat_type", "grove")
    pin = config.get("pin", 0)
    i2c_bus = config.get("i2c_bus", 1)
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating LIGHT readings without hardware")
        _sensor = EmulatedLightSensor()
    else:
        print(f"Initializing Grove Light Sensor on {hat_type} hat, channel {pin}...")
        _sensor = LightSensor(hat_type=hat_type, pin=pin, i2c_bus=i2c_bus)

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
