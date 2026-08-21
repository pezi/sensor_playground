"""
Sensor Playground Sensor Node — SHT11 / Sensirion SHT1x (Python)

Implements the Sensor Playground Sensor Interface on single-board computers
(Raspberry Pi & co.) with a Sensirion SHT1x sensor (temperature,
humidity) — the classic SHT10 / SHT11 / SHT15 family. The chips differ
only in calibration accuracy and speak the same proprietary two-wire
protocol (SCK + bidirectional DATA); it resembles I2C but is NOT I2C —
the sensor cannot share an I2C bus. Set "sensor_name" in config.json to
the chip on your board so the app shows the right name.

The protocol is implemented right here via lgpio bit-banging: the bus is
fully master-clocked with no minimum speed, so Python's pace is
harmless — the sensor simply waits between clock edges (unlike the DHT11,
whose reply timing a Python loop cannot follow). The DATA line is driven
open-drain style: released (input with pull-up) for a 1, actively pulled
LOW for a 0, because the sensor drives the same wire when answering.

The sensor must not be measured more than ~10% of the time or it heats
itself; the node reads at most every two seconds and serves the cached
values, and a failed read serves the last good reading for up to 30
seconds before read() reports a failure.

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

# -- SHT1x protocol constants (datasheet V5) ---------------------------------

CMD_TEMPERATURE = 0x03  # 000 00011: measure temperature (14 bit)
CMD_HUMIDITY = 0x05     # 000 00101: measure humidity (12 bit)

# A 14-bit measurement takes up to 320 ms.
MEASURE_TIMEOUT_S = 0.4

# 14-bit temperature at 3.3V supply: T = D1 + 0.01 * raw.
D1 = -39.66

# 12-bit humidity polynomial + temperature compensation.
C1 = -2.0468
C2 = 0.0367
C3 = -1.5955e-6
T1 = 0.01
T2 = 0.00008


def sht1x_temperature(raw):
    """Converts a raw 14-bit temperature reading to °C."""
    return D1 + T1 * raw


def sht1x_humidity(raw, temperature):
    """Converts a raw 12-bit humidity reading to %RH (compensated)."""
    linear = C1 + C2 * raw + C3 * raw * raw
    compensated = (temperature - 25.0) * (T1 + T2 * raw) + linear
    return max(0.0, min(100.0, compensated))


# -- Sensor ------------------------------------------------------------------


class Sht1xSensor:
    """Bit-bangs the SHT1x two-wire protocol on two GPIOs via lgpio.

    Polls are throttled to one hardware read every two seconds
    (self-heating limit). A failed read keeps the previous values; the
    cache goes stale — and read() reports a failure — only after
    _MAX_AGE_S without a successful read.
    """

    _MIN_INTERVAL_S = 2.0
    _MAX_AGE_S = 30.0

    def __init__(self, data_pin=4, sck_pin=5, gpio_chip=0, name="SHT11"):
        try:
            import lgpio
        except ImportError as e:
            # A raw ModuleNotFoundError here usually means the node runs
            # outside its venv, or the venv cannot see the system packages.
            raise RuntimeError(
                "lgpio is not installed. It ships as the system package "
                "python3-lgpio on Raspberry Pi OS — create this node's venv "
                "with --system-site-packages (see README, \"Setup\")."
            ) from e

        self._lgpio = lgpio
        self._handle = lgpio.gpiochip_open(gpio_chip)
        self._data = data_pin
        self._sck = sck_pin
        self.name = name
        self._cached = None
        self._cached_at = 0.0
        self._attempted_at = 0.0

        lgpio.gpio_claim_output(self._handle, self._sck, 0)
        self._data_release()
        # The sensor needs 11 ms after power-up before the first command.
        time.sleep(0.02)
        self._connection_reset()

    # -- line helpers: DATA is bidirectional and never driven HIGH --------

    def _data_release(self):
        """Releases DATA (pull-up makes it HIGH, the sensor may drive it)."""
        self._lgpio.gpio_claim_input(
            self._handle, self._data, self._lgpio.SET_PULL_UP
        )

    def _data_low(self):
        """Actively pulls DATA LOW."""
        self._lgpio.gpio_claim_output(self._handle, self._data, 0)

    def _data_read(self):
        """Samples the DATA line."""
        return self._lgpio.gpio_read(self._handle, self._data)

    def _sck_write(self, level):
        self._lgpio.gpio_write(self._handle, self._sck, level)

    def _sck_pulse(self):
        self._sck_write(1)
        self._sck_write(0)

    # -- protocol ---------------------------------------------------------

    def _transmission_start(self):
        """DATA falls and rises while SCK is high — the start pattern."""
        self._data_release()
        self._sck_write(0)
        self._sck_write(1)
        self._data_low()
        self._sck_write(0)
        self._sck_write(1)
        self._data_release()
        self._sck_write(0)

    def _connection_reset(self):
        """Resynchronises the interface: DATA high, nine or more clocks."""
        self._data_release()
        self._sck_write(0)
        for _ in range(10):
            self._sck_pulse()

    def _send_command(self, command):
        """Sends one command byte; returns False without the sensor's ACK."""
        self._transmission_start()
        for bit in range(7, -1, -1):
            if command & (1 << bit):
                self._data_release()
            else:
                self._data_low()
            self._sck_pulse()
        # ACK: the sensor pulls DATA low during the ninth clock.
        self._data_release()
        self._sck_write(1)
        acked = self._data_read() == 0
        self._sck_write(0)
        return acked

    def _read_byte(self, ack):
        """Reads one byte; [ack] keeps the transfer going, its absence ends
        it (the sensor then skips the CRC byte, which is not used here)."""
        value = 0
        self._data_release()
        for bit in range(7, -1, -1):
            self._sck_write(1)
            if self._data_read():
                value |= 1 << bit
            self._sck_write(0)
        if ack:
            self._data_low()
        else:
            self._data_release()
        self._sck_pulse()
        self._data_release()
        return value

    def _measure(self, command):
        """Runs one measurement; returns the raw result or None."""
        if not self._send_command(command):
            return None
        # The sensor releases DATA while measuring, pulls it low when done.
        deadline = time.monotonic() + MEASURE_TIMEOUT_S
        while self._data_read():
            if time.monotonic() > deadline:
                return None
            time.sleep(0.005)
        msb = self._read_byte(ack=True)
        lsb = self._read_byte(ack=False)
        return (msb << 8) | lsb

    # -- public API -------------------------------------------------------

    def read(self):
        """Return full-key readings for the REST API, or None when the
        sensor has not answered for a while."""
        now = time.monotonic()
        if now - self._attempted_at >= self._MIN_INTERVAL_S:
            self._attempted_at = now
            raw_temperature = self._measure(CMD_TEMPERATURE)
            raw_humidity = (
                self._measure(CMD_HUMIDITY)
                if raw_temperature is not None
                else None
            )
            if raw_temperature is None or raw_humidity is None:
                print("SHT1x read failed (serving cache)")
                self._connection_reset()
            else:
                temperature = sht1x_temperature(raw_temperature)
                self._cached = {
                    "temperature": round(temperature, 1),
                    "humidity": round(
                        sht1x_humidity(raw_humidity, temperature), 1
                    ),
                }
                self._cached_at = now

        if self._cached is None or now - self._cached_at > self._MAX_AGE_S:
            return None
        return self._cached

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        if full is None:
            return {}
        return {"temp": full["temperature"], "hum": full["humidity"]}

    def close(self):
        self._lgpio.gpiochip_close(self._handle)


class EmulatedSht1xSensor(Sht1xSensor):
    """Generates plausible SHT1x readings without hardware.

    A comfortable indoor climate: temperature and humidity drift on slow
    sines with different periods around 22 °C / 45 %RH, plus a little
    measurement noise.
    """

    def __init__(self, name="SHT11"):
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

    def close(self):
        pass


_sensor = None
_api_key = ""
_hostname = ""


# -- Main --------------------------------------------------------------------


def main():
    global _sensor, _api_key, _hostname

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    _hostname = config.get("hostname", "") or socket.gethostname()
    data_pin = config.get("data_pin", 4)
    sck_pin = config.get("sck_pin", 5)
    gpio_chip = config.get("gpio_chip", 0)
    sensor_name = config.get("sensor_name", "SHT11")
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print(f"Emulation mode: generating {sensor_name} readings without hardware")
        _sensor = EmulatedSht1xSensor(name=sensor_name)
    else:
        print(
            f"Initializing {sensor_name} sensor "
            f"(DATA=GPIO {data_pin}, SCK=GPIO {sck_pin})..."
        )
        _sensor = Sht1xSensor(
            data_pin=data_pin,
            sck_pin=sck_pin,
            gpio_chip=gpio_chip,
            name=sensor_name,
        )

    try:
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
    finally:
        _sensor.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
