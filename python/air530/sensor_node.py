"""
Sensor Tester Sensor Node — Air530 GPS (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a Grove GPS (Air530) module. Like the CozIR the
Air530 is not an I2C device: it continuously streams NMEA-0183 sentences
over a 9600-baud UART (serial). The node reads one burst per request and
parses the fix (port of the dart_periphery NmeaParser, see
serial_air530.dart):

- GGA sentences (preferred): latitude, longitude, MSL altitude, satellites
- GLL sentences (fallback): latitude, longitude only
- Sentences with bad checksums are skipped

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
import threading
import time
from functools import reduce
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- NMEA parsing (port of dart_periphery's NmeaParser) -----------------------


def _checksum_ok(line):
    """Verify the XOR checksum between '$' and '*'."""
    star = line.rfind("*")
    if star <= 0 or star + 3 > len(line):
        return False
    data = line[1:star]
    given = line[star + 1:]
    calc = reduce(lambda a, b: a ^ b, data.encode())
    return f"{calc:02X}" == given.upper()


def _coord_to_decimal(coord, hemi):
    """Convert NMEA ddmm.mmmm / dddmm.mmmm + hemisphere to decimal degrees."""
    if not coord or not hemi or "." not in coord:
        return None
    # Latitude has 2 degree digits, longitude 3 — infer from the hemisphere.
    deg_len = 3 if hemi in ("E", "W") else 2
    try:
        degrees = float(coord[:deg_len])
        minutes = float(coord[deg_len:])
    except ValueError:
        return None
    decimal = degrees + minutes / 60.0
    return -decimal if hemi in ("S", "W") else decimal


def _to_float(value):
    try:
        return float(value)
    except ValueError:
        return None


def _parse_gga(fields):
    """Parse a GGA sentence: lat, lon, MSL altitude, satellites in use."""
    if len(fields) < 10:
        return None
    fix_quality = fields[6]
    if not fix_quality.isdigit() or int(fix_quality) == 0:
        return None
    lat = _coord_to_decimal(fields[2], fields[3])
    lon = _coord_to_decimal(fields[4], fields[5])
    if lat is None or lon is None:
        return None
    fix = {"latitude": round(lat, 6), "longitude": round(lon, 6)}
    altitude = _to_float(fields[9])
    if altitude is not None:
        fix["altitude"] = round(altitude, 1)
    if fields[7].isdigit():
        fix["satellites"] = int(fields[7])
    return fix


def _parse_gll(fields):
    """Parse a GLL sentence: lat and lon only."""
    if len(fields) < 7 or not fields[6].startswith("A"):
        return None
    lat = _coord_to_decimal(fields[1], fields[2])
    lon = _coord_to_decimal(fields[3], fields[4])
    if lat is None or lon is None:
        return None
    return {"latitude": round(lat, 6), "longitude": round(lon, 6)}


def parse_nmea(block):
    """Parse a multi-line NMEA block; return the first valid fix or None.

    GGA fixes (with altitude and satellite count) are preferred over GLL.
    """
    gga_fix = None
    gll_fix = None
    for raw in block.splitlines():
        line = raw.strip()
        if not line.startswith("$") or "*" not in line:
            continue
        if not _checksum_ok(line):
            continue
        fields = line[1:line.index("*")].split(",")
        sentence = fields[0][-3:] if len(fields[0]) >= 5 else fields[0]
        if sentence == "GGA" and gga_fix is None:
            gga_fix = _parse_gga(fields)
        elif sentence == "GLL" and gll_fix is None:
            gll_fix = _parse_gll(fields)
    return gga_fix or gll_fix


# -- Sensor ------------------------------------------------------------------


class Air530Sensor:
    """Reads the position fix from an Air530 GPS streaming NMEA on a UART."""

    def __init__(self, serial_port="/dev/serial0"):
        import serial

        self.serial = serial.Serial(serial_port, 9600, timeout=1)
        self.name = "AIR530"
        self._lock = threading.Lock()

    def read(self):
        """Return full-key readings for the REST API, or None without a fix."""
        with self._lock:
            self.serial.reset_input_buffer()
            # One ~1 Hz NMEA burst; partial first lines fail the checksum
            # and are skipped by the parser.
            raw = self.serial.read(512).decode(errors="ignore")
        return parse_nmea(raw)

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        short = {"lat": full["latitude"], "lon": full["longitude"]}
        if "altitude" in full:
            short["alt"] = full["altitude"]
        if "satellites" in full:
            short["sats"] = full["satellites"]
        return short


class EmulatedAIR530Sensor(Air530Sensor):
    """Generates plausible Air530 GPS fixes without hardware.

    A receiver at St. Stephen's Cathedral in Vienna (48.2085 N, 16.3730 E):
    the position performs a tiny random walk around the base coordinate, the
    altitude drifts slowly around 171 m MSL and the satellite count varies
    between 4 and 12.
    """

    def __init__(self):
        self.name = "AIR530"
        self._lat = 48.2085
        self._lon = 16.3730

    def read(self):
        """Return full-key readings for the REST API, or None without a fix."""
        self._lat += random.uniform(-0.0001, 0.0001)
        self._lon += random.uniform(-0.0001, 0.0001)
        return {
            "latitude": round(self._lat, 6),
            "longitude": round(self._lon, 6),
            "altitude": round(171.0 + 5.0 * math.sin(time.time() / 120.0), 1),
            "satellites": random.randint(4, 12),
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
        print("Emulation mode: generating Air530 readings without hardware")
        _sensor = EmulatedAIR530Sensor()
    else:
        print(f"Initializing Air530 GPS on {serial_port}...")
        _sensor = Air530Sensor(serial_port=serial_port)

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
    # allow_empty: a GPS with no fix yet is not an error, just a normal
    # warm-up state, so serve 200 with a metadata-only body (no latitude/
    # longitude) exactly as the ESP32 sketch does. The app then shows its
    # "waiting for satellite fix" screen; a 503 would instead be read as an
    # API/network failure.
    wifi.run_rest_server(
        _sensor.name,
        _api_key,
        _hostname,
        _sensor.read,
        ssl_cert,
        ssl_key,
        allow_empty=True,
    )


if __name__ == "__main__":
    main()
