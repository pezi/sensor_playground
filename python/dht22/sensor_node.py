"""
Sensor Playground Sensor Node — DHT22 / Grove Temperature & Humidity Pro (Python)

Implements the Sensor Playground Sensor Interface on single-board computers
(Raspberry Pi & co.) with a DHT22 sensor (temperature, humidity) — the
white Grove Temperature & Humidity Sensor Pro module. The DHT11 (blue
module) speaks the same single-wire protocol with coarser resolution and
a narrower range; it has its own node in ../dht11/.

The single-wire protocol is timing-critical (26-70 µs pulses), so the
pin is read through adafruit-circuitpython-dht rather than gpiozero —
the library ships a small compiled pulse reader that a Python loop
cannot replace. Reads occasionally fail even on a healthy sensor; the
node retries and serves the last good reading for up to 30 seconds, so
a single failed read does not surface as an error.

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


class Dht22Sensor:
    """Reads temperature and humidity from a DHT22 (AM2302).

    The chip samples at most once every two seconds, so polls are
    throttled to one hardware read in that interval. A failed read keeps
    the previous values; the cache goes stale — and read() reports a
    failure — only after _MAX_AGE_S without a successful read.
    """

    _MIN_INTERVAL_S = 2.0
    _MAX_AGE_S = 30.0

    def __init__(self, gpio_pin=4, name="DHT22"):
        import adafruit_dht
        import board

        pin = getattr(board, f"D{gpio_pin}")
        # use_pulseio=True (the default) starts the bundled compiled pulse
        # reader, the only way to hit the single-wire timing from Python.
        self._dht = adafruit_dht.DHT22(pin)
        self.name = name
        self._cached = None
        self._cached_at = 0.0
        self._attempted_at = 0.0

    def read(self):
        """Return full-key readings for the REST API, or None when the
        sensor has not answered for a while."""
        now = time.monotonic()
        if now - self._attempted_at >= self._MIN_INTERVAL_S:
            self._attempted_at = now
            try:
                temperature = self._dht.temperature
                humidity = self._dht.humidity
                if temperature is not None and humidity is not None:
                    self._cached = {
                        "temperature": round(float(temperature), 1),
                        "humidity": round(float(humidity), 1),
                    }
                    self._cached_at = now
            except RuntimeError as exc:
                # Single-wire reads fail now and then; the cache covers it.
                print(f"DHT read failed (retrying): {exc}")

        if self._cached is None or now - self._cached_at > self._MAX_AGE_S:
            return None
        return self._cached

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        return {"temp": full["temperature"], "hum": full["humidity"]}


class EmulatedDht22Sensor(Dht22Sensor):
    """Generates plausible DHT22 readings without hardware.

    A comfortable indoor climate: temperature and humidity drift on slow
    sines with different periods around 22 °C / 45 %RH, rounded to the
    DHT22's one-decimal resolution.
    """

    def __init__(self, name="DHT22"):
        self.name = name

    def read(self):
        t = time.time()
        return {
            "temperature": round(22.0 + 2.0 * math.sin(t / 60.0), 1),
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
    gpio_pin = config.get("gpio_pin", 4)
    sensor_name = config.get("sensor_name", "DHT22")
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print(f"Emulation mode: generating {sensor_name} readings without hardware")
        _sensor = EmulatedDht22Sensor(name=sensor_name)
    else:
        print(f"Initializing {sensor_name} sensor on GPIO {gpio_pin}...")
        _sensor = Dht22Sensor(gpio_pin=gpio_pin, name=sensor_name)

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
