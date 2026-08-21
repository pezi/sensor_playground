"""
Sensor Playground Sensor Node — Grove Capacitive Moisture Sensor (Python)

Implements the Sensor Playground Sensor Interface on single-board computers with a
Grove Capacitive Moisture Sensor (Corrosion-Resistant) — an analog probe whose
output voltage falls as the soil gets wetter. It reports the soil moisture as
a percentage (JSON key `moisture`) mapped linearly between the two calibration
points in config.json (raw ADC when dry vs. when wet), alongside the raw
reading (`adc`/`adcMax`) for calibrating them.
https://wiki.seeedstudio.com/Grove-Capacitive_Moisture_Sensor-Corrosion-Resistant/

The Raspberry Pi has no analog input, so an analog sensor needs an extension
hat with an ADC. This node reads it through the sibling `extension_hat`
helper — there is no direct-GPIO option (unlike the digital contact node):

    "hat_type": "grove"       Seeed Grove Base Hat (12-bit ADC, 0-4095)
    "hat_type": "nano"        FriendlyARM NanoHat Hub (10-bit, 0-1023)
    "hat_type": "grovePlus"   Seeed GrovePi+ (10-bit, 0-1023)

- HTTPS REST API on port 9132 + UDP discovery on port 9133 (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import math
import socket
import sys
import time
from pathlib import Path


# The extension_hat helper and the shared BLE transport live in sibling folders.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "extension_hat"))
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# Averaging window; a single ADC read of the probe is noisy.
SAMPLE_COUNT = 4

# -- Sensor ------------------------------------------------------------------


class MoistureSensor:
    """Reads the probe through an extension hat's ADC and maps the raw value
    onto 0-100 % between the dry and wet calibration points."""

    def __init__(self, adc_dry, adc_wet, hat_type="grove", pin=0, i2c_bus=1):
        from extension_hat import GroveBaseHat, GrovePiPlusHat, NanoHatHub

        if adc_dry == adc_wet:
            raise ValueError("adc_dry and adc_wet must differ (calibrate!)")

        if hat_type == "grove":
            self._hat = GroveBaseHat(i2c_bus)
            self._read = lambda: self._hat.read_adc_raw(pin)
            self.adc_max = 4095
        elif hat_type == "nano":
            self._hat = NanoHatHub(i2c_bus)
            self._read = lambda: self._hat.analog_read(pin)
            self.adc_max = 1023
        elif hat_type == "grovePlus":
            self._hat = GrovePiPlusHat(i2c_bus)
            self._read = lambda: self._hat.analog_read(pin)
            self.adc_max = 1023
        else:
            raise ValueError(f"Unknown hat_type {hat_type!r}")

        self._adc_dry = adc_dry
        self._adc_wet = adc_wet
        self.name = "MOISTURE"

    def _read_averaged(self):
        total = 0
        for _ in range(SAMPLE_COUNT):
            total += self._read()
            time.sleep(0.002)
        return round(total / SAMPLE_COUNT)

    def _percent(self, raw):
        span = self._adc_dry - self._adc_wet
        percent = 100 * (self._adc_dry - raw) / span
        return round(min(100, max(0, percent)))

    def read(self):
        """Return full-key readings for the REST API."""
        raw = self._read_averaged()
        # The raw reading and its full scale help calibrate adc_dry/adc_wet.
        return {
            "moisture": self._percent(raw),
            "adc": raw,
            "adcMax": self.adc_max,
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        return {"moist": self._percent(self._read_averaged())}


class EmulatedMoistureSensor(MoistureSensor):
    """Generates plausible soil-moisture readings without hardware.

    A watering cycle: the moisture slowly dries from ~85 % down to ~25 %
    and jumps back up, on a few-minute loop for easy demoing.
    """

    def __init__(self):
        self.name = "MOISTURE"
        self.adc_max = 4095
        self._adc_dry = 2600
        self._adc_wet = 1100

    def _read_averaged(self):
        cycle = (time.time() % 300) / 300  # 0 → 1 over five minutes
        percent = 85 - 60 * cycle + 2 * math.sin(time.time() / 3)
        raw = self._adc_dry - (self._adc_dry - self._adc_wet) * percent / 100
        return round(raw)


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
    adc_dry = config.get("adc_dry", 2600)
    adc_wet = config.get("adc_wet", 1100)
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating MOISTURE readings without hardware")
        _sensor = EmulatedMoistureSensor()
    else:
        print(
            f"Initializing moisture probe on {hat_type} hat, channel {pin} "
            f"(dry={adc_dry}, wet={adc_wet})..."
        )
        _sensor = MoistureSensor(
            adc_dry, adc_wet, hat_type=hat_type, pin=pin, i2c_bus=i2c_bus
        )

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
