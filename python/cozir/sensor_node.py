"""
Sensor Tester Sensor Node — CozIR CO2 Sensor (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a CozIR-A sensor (temperature, humidity, CO2).
Unlike the other environment sensors the CozIR is not an I2C device: it
talks a simple ASCII command protocol over a 9600-baud UART (serial).

Protocol (see the dart_periphery serial_cozir.dart example):
    M 4164\r\n   select humidity, temperature and CO2 output fields
    K 2\r\n      polling mode
    Q\r\n        request one measurement:
                 "H 00495 T 01234 Z 06399" -> 49.5 %RH, 23.4 degC, 639.9 ppm

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
import re
import socket
import sys
import threading
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Sensor ------------------------------------------------------------------

# One measurement line: "H 00495 T 01234 Z 06399" (a leading space may occur).
MEASUREMENT_PATTERN = re.compile(r"H\s+(\d+)\s+T\s+(\d+)\s+Z\s+(\d+)")


class CozirSensor:
    """Reads temperature, humidity, and CO2 from a CozIR over a UART."""

    def __init__(self, serial_port="/dev/serial0"):
        import serial

        self.serial = serial.Serial(serial_port, 9600, timeout=1)
        self.name = "COZIR"
        self._lock = threading.Lock()
        # Select the humidity, temperature and CO2 output fields, then
        # switch to polling mode (one measurement per Q command).
        self.serial.write(b"M 4164\r\n")
        self.serial.write(b"K 2\r\n")
        self.serial.reset_input_buffer()

    def read(self):
        """Return full-key readings for the REST API."""
        with self._lock:
            self.serial.reset_input_buffer()
            self.serial.write(b"Q\r\n")
            raw = self.serial.read_until(b"\n", 64).decode(errors="ignore")
        match = MEASUREMENT_PATTERN.search(raw)
        if match is None:
            return None
        return {
            "temperature": round((int(match.group(2)) - 1000) / 10.0, 1),
            "humidity": round(int(match.group(1)) / 10.0, 1),
            "co2": round(int(match.group(3)) / 10.0, 1),
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


class EmulatedCozirSensor(CozirSensor):
    """Generates plausible CozIR readings without hardware.

    A quiet indoor room: temperature and humidity follow slow sines with
    different periods, the CO2 concentration does a bounded random walk
    between 400 and 1500 ppm.
    """

    def __init__(self):
        self.name = "COZIR"
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
    serial_port = config.get("serial_port", "/dev/serial0")
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating CozIR readings without hardware")
        _sensor = EmulatedCozirSensor()
    else:
        print(f"Initializing CozIR sensor on {serial_port}...")
        _sensor = CozirSensor(serial_port=serial_port)

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
