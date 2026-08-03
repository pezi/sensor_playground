"""
Sensor Tester Sensor Node — TSL2591 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a TSL2591 I2C sensor (visible light, infrared,
illuminance).

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

# Counts at which the current gain is judged too high or too low. The ADC is
# 16-bit, so a saturated channel reads 0xFFFF and the lux value is invalid.
SATURATION_COUNTS = 0xFFFF
TOO_DARK_COUNTS = 100

# -- Sensor ------------------------------------------------------------------


class TSL2591Sensor:
    """Reads visible light, infrared and lux from a TSL2591.

    The chip has two photodiodes: channel 0 is broadband (visible + IR) and
    channel 1 is infrared only. Neither is "visible light" on its own — the
    difference of the two is. Lux is a third thing again, derived from both
    channels together with the gain and integration time.
    """

    #: Gain names accepted in config.json, mapped to the driver's constants
    #: lazily (the driver is only importable where the hardware library is).
    GAIN_NAMES = ("low", "med", "high", "max")

    def __init__(self, i2c_bus=1, gain="med", integration_ms=300, auto_gain=True):
        # Adafruit Blinka's `board` module has no pin map for many SBCs
        # (e.g. the NanoPi NEO2), so open the Linux I2C bus by number via
        # adafruit-extended-bus, honoring the configured i2c_bus.
        import adafruit_tsl2591
        from adafruit_extended_bus import ExtendedI2C

        self._driver = adafruit_tsl2591
        self._gains = [
            adafruit_tsl2591.GAIN_LOW,
            adafruit_tsl2591.GAIN_MED,
            adafruit_tsl2591.GAIN_HIGH,
            adafruit_tsl2591.GAIN_MAX,
        ]
        self._integrations = {
            100: adafruit_tsl2591.INTEGRATIONTIME_100MS,
            200: adafruit_tsl2591.INTEGRATIONTIME_200MS,
            300: adafruit_tsl2591.INTEGRATIONTIME_300MS,
            400: adafruit_tsl2591.INTEGRATIONTIME_400MS,
            500: adafruit_tsl2591.INTEGRATIONTIME_500MS,
            600: adafruit_tsl2591.INTEGRATIONTIME_600MS,
        }

        self.sensor = adafruit_tsl2591.TSL2591(ExtendedI2C(i2c_bus))
        self._gain_index = self._gain_index_for(gain)
        self.sensor.gain = self._gains[self._gain_index]
        self.sensor.integration_time = self._integration_for(integration_ms)
        self._auto_gain = auto_gain
        self.name = "TSL2591"

    def _gain_index_for(self, gain):
        name = str(gain).lower()
        if name not in self.GAIN_NAMES:
            raise ValueError(
                f"gain must be one of {', '.join(self.GAIN_NAMES)}, got {gain!r}"
            )
        return self.GAIN_NAMES.index(name)

    def _integration_for(self, integration_ms):
        try:
            return self._integrations[int(integration_ms)]
        except (KeyError, TypeError, ValueError):
            valid = ", ".join(str(ms) for ms in sorted(self._integrations))
            raise ValueError(
                f"integration_ms must be one of {valid}, got {integration_ms!r}"
            ) from None

    def _auto_gain_step(self, broadband):
        """Move one gain step when the broadband channel pins at either end.

        Only one step per reading: changing the gain invalidates the
        integration already in flight, so the *next* reading is the one that
        benefits. Jumping straight to the extreme instead would make the value
        oscillate whenever the light sits near a threshold.
        """
        if not self._auto_gain:
            return
        index = self._gain_index
        if broadband >= SATURATION_COUNTS:
            index = max(0, index - 1)
        elif broadband <= TOO_DARK_COUNTS:
            index = min(len(self._gains) - 1, index + 1)
        if index != self._gain_index:
            self._gain_index = index
            self.sensor.gain = self._gains[index]

    def read(self):
        """Return full-key readings for the REST API."""
        # One read of both channels: the driver's own `visible` property
        # subtracts the IR channel from the *packed 32-bit* value rather than
        # from channel 0, so compute it here the way the ESP32 sketch does and
        # keep the two nodes reporting identical numbers.
        broadband, infrared = self.sensor.raw_luminosity

        # Floored: in near-darkness noise can put IR marginally above the
        # broadband channel, and a negative amount of visible light is
        # meaningless on a chart.
        visible = max(0, broadband - infrared)

        try:
            lux = round(self.sensor.lux, 1)
        except RuntimeError:
            # A channel saturated. The counts still show the app that it is
            # very bright; omit the lux rather than publish a wrong value —
            # the metric registry renders whatever keys are present.
            lux = None

        self._auto_gain_step(broadband)

        reading = {"visible": visible, "ir": infrared}
        if lux is not None:
            reading["lux"] = lux
        return reading

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        short = {"vis": full["visible"], "ir": full["ir"]}
        if "lux" in full:
            short["lux"] = full["lux"]
        return short


class EmulatedTSL2591Sensor(TSL2591Sensor):
    """Generates plausible TSL2591 readings without hardware.

    A lit room near a window: a few hundred lux drifting on a slow cycle, with
    the infrared channel holding the roughly one-third share of the broadband
    count that daylight and incandescent lamps produce.
    """

    def __init__(self):
        self.name = "TSL2591"

    def read(self):
        t = time.time()
        broadband = round(5400 + 1200 * math.sin(t / 90.0) + random.uniform(-40, 40))
        infrared = round(broadband * 0.28 + random.uniform(-20, 20))
        return {
            "visible": max(0, broadband - infrared),
            "ir": infrared,
            "lux": round(310 + 70 * math.sin(t / 90.0) + random.uniform(-2, 2), 1),
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
        print("Emulation mode: generating TSL2591 readings without hardware")
        _sensor = EmulatedTSL2591Sensor()
    else:
        print(f"Initializing TSL2591 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = TSL2591Sensor(
            i2c_bus=i2c_bus,
            gain=config.get("gain", "med"),
            integration_ms=config.get("integration_ms", 300),
            auto_gain=config.get("auto_gain", True),
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
