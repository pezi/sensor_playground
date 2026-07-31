"""
Sensor Tester Sensor Node — BME680 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a BME680 I2C sensor (temperature, humidity,
pressure, IAQ).

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
from collections import deque
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Sensor ------------------------------------------------------------------


class BME680Sensor:
    """Reads temperature, humidity, pressure, and IAQ from a BME680."""

    _GAS_BURN_IN = 50
    _HUMIDITY_BASELINE = 40.0
    _HUMIDITY_WEIGHT = 0.25

    def __init__(self, i2c_bus=1):
        import bme680
        from smbus2 import SMBus

        self.sensor = bme680.BME680(bme680.I2C_ADDR_PRIMARY, SMBus(i2c_bus))
        self.sensor.set_humidity_oversample(bme680.OS_2X)
        self.sensor.set_pressure_oversample(bme680.OS_4X)
        self.sensor.set_temperature_oversample(bme680.OS_8X)
        self.sensor.set_filter(bme680.FILTER_SIZE_3)
        self.sensor.set_gas_status(bme680.ENABLE_GAS_MEAS)
        self.sensor.set_gas_heater_temperature(320)
        self.sensor.set_gas_heater_duration(150)
        self._gas_data = deque([0] * self._GAS_BURN_IN, maxlen=self._GAS_BURN_IN)
        self._last_iaq = 0
        self.name = "BME680"

    def read(self):
        """Return full-key readings for the REST API."""
        if not self.sensor.get_sensor_data():
            return None
        data = self.sensor.data
        return {
            "temperature": round(data.temperature, 1),
            "humidity": round(data.humidity, 1),
            "pressure": round(data.pressure, 2),
            "iaq": self._calculate_iaq(
                int(data.gas_resistance), data.humidity
            ),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        return {
            "temp": full["temperature"],
            "hum": full["humidity"],
            "press": full["pressure"],
            "iaq": full["iaq"],
        }

    def _calculate_iaq(self, gas_resistance, humidity):
        """Calculate IAQ from a rolling gas-resistance baseline and humidity.

        Algorithm ported from dart_periphery BME680 driver.
        Uses a rolling window of 50 readings to establish the gas baseline,
        then scores gas (75%) and humidity (25%) relative to their baselines.
        """
        try:
            self._gas_data.append(gas_resistance)
            gas_baseline = round(sum(self._gas_data) / self._GAS_BURN_IN)

            gas_offset = gas_baseline - gas_resistance
            hum_offset = humidity - self._HUMIDITY_BASELINE

            if hum_offset > 0:
                hum_score = (
                    (100.0 - self._HUMIDITY_BASELINE - hum_offset)
                    / (100.0 - self._HUMIDITY_BASELINE)
                    * (self._HUMIDITY_WEIGHT * 100.0)
                )
            else:
                hum_score = (
                    (self._HUMIDITY_BASELINE + hum_offset)
                    / self._HUMIDITY_BASELINE
                    * (self._HUMIDITY_WEIGHT * 100.0)
                )

            gas_weight = 100.0 - (self._HUMIDITY_WEIGHT * 100.0)
            if gas_offset > 0:
                gas_score = (gas_resistance / gas_baseline) * gas_weight
            else:
                gas_score = gas_weight

            self._last_iaq = round(hum_score + gas_score)
            return self._last_iaq
        except (ZeroDivisionError, ValueError):
            return self._last_iaq


class EmulatedBME680Sensor(BME680Sensor):
    """Generates plausible BME680 readings without hardware.

    A quiet indoor room: temperature, humidity and pressure follow slow
    sines with different periods, the gas resistance does a bounded random
    walk around 120 kOhm and the IAQ score is computed from it with the
    real rolling-baseline algorithm.
    """

    def __init__(self):
        self.name = "BME680"
        self._gas_data = deque([0] * self._GAS_BURN_IN, maxlen=self._GAS_BURN_IN)
        self._last_iaq = 0
        self._gas = 120000.0

    def read(self):
        t = time.time()
        self._gas = min(max(self._gas + random.uniform(-2000, 2000), 20000.0), 500000.0)
        humidity = 45.0 + 8.0 * math.sin(t / 97.0) + random.uniform(-0.5, 0.5)
        return {
            "temperature": round(
                22.0 + 2.0 * math.sin(t / 60.0) + random.uniform(-0.1, 0.1), 1
            ),
            "humidity": round(humidity, 1),
            "pressure": round(
                1013.0 + 3.0 * math.sin(t / 300.0) + random.uniform(-0.2, 0.2), 2
            ),
            "iaq": self._calculate_iaq(int(self._gas), humidity),
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
        print("Emulation mode: generating BME680 readings without hardware")
        _sensor = EmulatedBME680Sensor()
    else:
        print(f"Initializing BME680 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = BME680Sensor(i2c_bus=i2c_bus)

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
    main()
