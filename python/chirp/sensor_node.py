"""
Sensor Playground Sensor Node — Chirp I2C Soil Moisture Sensor (Python)

Implements the Sensor Playground Sensor Interface on single-board computers with a
Catnip Electronics I2C Soil Moisture Sensor (the "Chirp" sensor, default I2C
address 0x20). It reports the soil moisture as a percentage (JSON key
`moisture`, mapped linearly between the two capacitance calibration points in
config.json), the soil temperature (`temperature`) and the ambient light
level (`light`, raw brightness counts, higher = brighter), alongside the raw
capacitance (`cap`) for calibrating.

- Register protocol: https://github.com/Apollon77/I2CSoilMoistureSensor
- Raspberry Pi reference: https://github.com/ageir/chirp-rpi

The chip measures light by timing a phototransistor discharge, which takes up
to three seconds — so the light value is harvested from a measurement started
on an earlier read, and the `light` key is absent until the first one
completes (a few seconds after start).

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


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Chirp register map (see the Apollon77 library) --------------------------

REG_GET_CAPACITANCE = 0x00
REG_MEASURE_LIGHT = 0x03
REG_GET_LIGHT = 0x04
REG_GET_TEMPERATURE = 0x05
REG_RESET = 0x06
REG_GET_VERSION = 0x07

# A light measurement takes up to three seconds on the chip.
LIGHT_MEASURE_SECONDS = 3.0

# -- Sensor ------------------------------------------------------------------


class ChirpSensor:
    """Reads the Chirp sensor over I2C and maps the raw capacitance onto
    0-100 % between the dry and wet calibration points."""

    def __init__(self, cap_dry, cap_wet, i2c_bus=1, address=0x20):
        from smbus2 import SMBus

        if cap_dry == cap_wet:
            raise ValueError("cap_dry and cap_wet must differ (calibrate!)")

        self._bus = SMBus(i2c_bus)
        self._address = address
        self._cap_dry = cap_dry
        self._cap_wet = cap_wet
        self._light = None
        self._light_started = None
        self.name = "CHIRP"

        self._bus.write_byte(self._address, REG_RESET)
        time.sleep(1.0)  # the chip needs a moment after a reset
        version = self._read_u16(REG_GET_VERSION) & 0xFF
        print(f"Chirp sensor at 0x{address:02x}, firmware version 0x{version:02x}")

    def _read_u16(self, register):
        data = self._bus.read_i2c_block_data(self._address, register, 2)
        return (data[0] << 8) | data[1]

    def _percent(self, capacitance):
        span = self._cap_wet - self._cap_dry
        percent = 100 * (capacitance - self._cap_dry) / span
        return round(min(100, max(0, percent)))

    def _temperature(self):
        raw = self._read_u16(REG_GET_TEMPERATURE)
        if raw >= 0x8000:
            raw -= 0x10000  # the chip reports a signed 16-bit value
        return round(raw / 10.0, 1)

    def _update_light(self):
        """Harvests a finished light measurement and starts the next one.

        The measurement runs on the chip, so this never blocks; the first
        call only starts one and leaves the value None.
        """
        now = time.monotonic()
        if (
            self._light_started is not None
            and now - self._light_started >= LIGHT_MEASURE_SECONDS
        ):
            raw = self._read_u16(REG_GET_LIGHT)
            self._light = 65535 - raw  # the chip counts up in darkness
            self._light_started = None
        if self._light_started is None:
            self._bus.write_byte(self._address, REG_MEASURE_LIGHT)
            self._light_started = now

    def read(self):
        """Return full-key readings for the REST API."""
        self._update_light()
        capacitance = self._read_u16(REG_GET_CAPACITANCE)
        reading = {
            "moisture": self._percent(capacitance),
            "temperature": self._temperature(),
            # The raw capacitance helps calibrate cap_dry/cap_wet.
            "cap": capacitance,
        }
        if self._light is not None:
            reading["light"] = self._light
        return reading

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        short = {"moist": full["moisture"], "temp": full["temperature"]}
        if "light" in full:
            short["light"] = full["light"]
        return short


class EmulatedChirpSensor(ChirpSensor):
    """Generates plausible Chirp readings without hardware.

    A watering cycle for the moisture, a steady room temperature and a slow
    day/night curve for the light.
    """

    def __init__(self):
        self._cap_dry = 290
        self._cap_wet = 520
        self._light = None
        self._light_started = None
        self.name = "CHIRP"

    def _read_u16(self, register):
        t = time.time()
        if register == REG_GET_CAPACITANCE:
            cycle = (t % 300) / 300  # rewatered every five minutes
            percent = 85 - 60 * cycle + 2 * math.sin(t / 3)
            return round(self._cap_dry + (self._cap_wet - self._cap_dry) * percent / 100)
        if register == REG_GET_TEMPERATURE:
            return round(10 * (21.5 + 1.5 * math.sin(t / 60)))
        if register == REG_GET_LIGHT:
            return round(20000 + 15000 * math.sin(t / 120))
        return 0

    def _update_light(self):
        self._light = 65535 - self._read_u16(REG_GET_LIGHT)


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
    address = config.get("address", 0x20)
    cap_dry = config.get("cap_dry", 290)
    cap_wet = config.get("cap_wet", 520)
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating CHIRP readings without hardware")
        _sensor = EmulatedChirpSensor()
    else:
        print(
            f"Initializing Chirp sensor on I2C bus {i2c_bus} "
            f"(dry={cap_dry}, wet={cap_wet})..."
        )
        _sensor = ChirpSensor(cap_dry, cap_wet, i2c_bus=i2c_bus, address=address)

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
