"""
Sensor Playground Sensor Node — SparkFun ISL29125 RGB Light Sensor (Python)

Implements the Sensor Playground Sensor Interface on single-board computers with
a SparkFun RGB Light Sensor breakout — an Intersil/Renesas ISL29125 (I2C
address 0x44) measuring the intensity of red, green and blue light while
rejecting infrared. The channels are normalized against the brightest one so
the app can show the measured color directly (JSON keys `red`/`green`/`blue`,
0-255, absent in complete darkness); an approximate illuminance (`lux`) is
derived from the green channel, whose spectral response resembles the human
eye. There is no clear channel, so unlike the TCS34725 no color temperature
is reported.
https://www.sparkfun.com/sparkfun-rgb-light-sensor-isl29125.html

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

# -- ISL29125 register map (see the SparkFun library / datasheet) ------------

REG_DEVICE_ID = 0x00      # reads 0x7D; writing 0x46 resets the chip
REG_CONFIG1 = 0x01
REG_CONFIG2 = 0x02
REG_CONFIG3 = 0x03
REG_GREEN_LOW = 0x09      # G L/H, R L/H, B L/H — six consecutive bytes

DEVICE_ID = 0x7D
RESET_COMMAND = 0x46

# CONFIG1: RGB sampling mode (0x05) in the 10,000 lux range (0x08), 16-bit.
CONFIG1_RGB_10KLUX = 0x0D
# CONFIG2: IR compensation on, maximum adjustment (the SparkFun default).
CONFIG2_IR_ADJUST_HIGH = 0xBF
CONFIG3_NO_INTERRUPTS = 0x00

# Approximate green-counts-to-lux factor for the 10K range at 16 bits.
LUX_PER_COUNT = 10000.0 / 65535.0

# -- Sensor ------------------------------------------------------------------


class Isl29125Sensor:
    """Reads the ISL29125 over I2C and derives color and illuminance."""

    def __init__(self, i2c_bus=1, address=0x44):
        from smbus2 import SMBus

        self._bus = SMBus(i2c_bus)
        self._address = address
        self.name = "ISL29125"

        device_id = self._bus.read_byte_data(address, REG_DEVICE_ID)
        if device_id != DEVICE_ID:
            raise RuntimeError(
                f"No ISL29125 at 0x{address:02x} "
                f"(device ID 0x{device_id:02x}, expected 0x{DEVICE_ID:02x})"
            )
        self._bus.write_byte_data(address, REG_DEVICE_ID, RESET_COMMAND)
        time.sleep(0.1)
        self._bus.write_byte_data(address, REG_CONFIG1, CONFIG1_RGB_10KLUX)
        self._bus.write_byte_data(address, REG_CONFIG2, CONFIG2_IR_ADJUST_HIGH)
        self._bus.write_byte_data(address, REG_CONFIG3, CONFIG3_NO_INTERRUPTS)
        # One conversion takes ~100 ms per channel; let the first RGB
        # sampling cycle complete before serving readings.
        time.sleep(0.4)

    def _read_channels(self):
        """Returns the raw 16-bit (green, red, blue) counts."""
        data = self._bus.read_i2c_block_data(self._address, REG_GREEN_LOW, 6)
        green = data[0] | (data[1] << 8)
        red = data[2] | (data[3] << 8)
        blue = data[4] | (data[5] << 8)
        return green, red, blue

    def read(self):
        """Return full-key readings for the REST API."""
        green, red, blue = self._read_channels()
        reading = {"lux": round(green * LUX_PER_COUNT)}
        brightest = max(red, green, blue)
        if brightest > 0:
            # Normalize against the brightest channel so the app can show
            # the color; in complete darkness there is no color to report.
            reading["red"] = round(255 * red / brightest)
            reading["green"] = round(255 * green / brightest)
            reading["blue"] = round(255 * blue / brightest)
        return reading

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        green, _, _ = self._read_channels()
        return {"lux": round(green * LUX_PER_COUNT)}


class EmulatedIsl29125Sensor(Isl29125Sensor):
    """Generates plausible ISL29125 readings without hardware.

    Indoor light slowly shifting between warm and cool white, with the
    brightness breathing over a couple of minutes.
    """

    def __init__(self):
        self.name = "ISL29125"

    def _read_channels(self):
        t = time.time()
        brightness = 0.35 + 0.3 * math.sin(t / 120.0)
        warmth = 0.5 + 0.5 * math.sin(t / 45.0)
        red = 65535 * brightness * (0.6 + 0.4 * warmth)
        green = 65535 * brightness
        blue = 65535 * brightness * (1.0 - 0.5 * warmth)
        return round(green), round(red), round(blue)


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
    address = config.get("address", 0x44)
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating ISL29125 readings without hardware")
        _sensor = EmulatedIsl29125Sensor()
    else:
        print(f"Initializing ISL29125 on I2C bus {i2c_bus}, address 0x{address:02x}...")
        _sensor = Isl29125Sensor(i2c_bus=i2c_bus, address=address)

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
