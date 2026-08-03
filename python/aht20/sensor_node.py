"""
Sensor Tester Sensor Node — AHT10/AHT20 (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with an ASAIR AHT10 or AHT20 I2C sensor
(temperature, humidity). Both chips share the fixed address 0x38 and the
same measurement protocol.

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


class AHT20Sensor:
    """Reads temperature and humidity from an AHT10/AHT20.

    Protocol (identical on both chips): trigger a measurement with
    0xAC 0x33 0x00, wait ~80 ms, then read 6 bytes — a status byte
    followed by 20-bit humidity and 20-bit temperature.
    Only the calibrate opcode differs (AHT20: 0xBE, AHT10: 0xE1),
    so initialization tries both.
    """

    _CMD_CALIBRATE_AHT20 = [0xBE, 0x08, 0x00]
    _CMD_CALIBRATE_AHT10 = [0xE1, 0x08, 0x00]
    _CMD_MEASURE = [0xAC, 0x33, 0x00]
    _STATUS_BUSY = 0x80

    def __init__(self, i2c_bus=1, address=0x38, name="AHT20"):
        from smbus2 import SMBus, i2c_msg

        self.bus = SMBus(i2c_bus)
        self.address = address
        self._i2c_msg = i2c_msg
        self.name = name
        time.sleep(0.04)
        self._calibrate()

    def _calibrate(self):
        """Send the calibrate command; tolerate either chip variant."""
        for command in (self._CMD_CALIBRATE_AHT20, self._CMD_CALIBRATE_AHT10):
            try:
                self.bus.i2c_rdwr(self._i2c_msg.write(self.address, command))
                time.sleep(0.01)
                return
            except OSError:
                continue

    def read(self):
        """Return full-key readings for the REST API, or None when busy."""
        self.bus.i2c_rdwr(self._i2c_msg.write(self.address, self._CMD_MEASURE))
        time.sleep(0.08)
        msg = self._i2c_msg.read(self.address, 6)
        self.bus.i2c_rdwr(msg)
        raw = list(msg)
        if raw[0] & self._STATUS_BUSY:
            return None
        hum_raw = (raw[1] << 12) | (raw[2] << 4) | (raw[3] >> 4)
        temp_raw = ((raw[3] & 0x0F) << 16) | (raw[4] << 8) | raw[5]
        return {
            "temperature": round(temp_raw / 1048576.0 * 200.0 - 50.0, 1),
            "humidity": round(hum_raw / 1048576.0 * 100.0, 1),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        return {"temp": full["temperature"], "hum": full["humidity"]}


class EmulatedAHT20Sensor(AHT20Sensor):
    """Generates plausible AHT10/AHT20 readings without hardware.

    A comfortable indoor climate: temperature and humidity drift on slow
    sines with different periods around 22 °C / 45 %RH, plus a little
    measurement noise.
    """

    def __init__(self, name="AHT20"):
        self.name = name

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
    sensor_name = config.get("sensor_name", "AHT20")
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print(f"Emulation mode: generating {sensor_name} readings without hardware")
        _sensor = EmulatedAHT20Sensor(name=sensor_name)
    else:
        print(f"Initializing {sensor_name} sensor on /dev/i2c-{i2c_bus}...")
        _sensor = AHT20Sensor(i2c_bus=i2c_bus, name=sensor_name)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_poll(
            _sensor.name,
            _api_key,
            lambda: {
                "sensor": _sensor.name,
                "host": _hostname,
                **(_sensor.read() or {}),
            },
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
