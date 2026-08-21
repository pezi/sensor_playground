"""
Sensor Playground Sensor Node — Grove Ultrasonic Ranger (Python)

Implements the *push* variant of the Sensor Playground Sensor Interface on
single-board computers (Raspberry Pi & co.) with a Grove Ultrasonic
Ranger (40 kHz sonar, 2 cm - 3.5 m). Like the VL53L0X node it measures
continuously and pushes one JSON message ({"distance": <mm>}, null when
no echo returns) whenever the distance changes, or at least once per
second as a heartbeat.

The Grove ranger uses a single SIG pin for both trigger and echo —
unlike the common HC-SR04 with separate TRIG/ECHO pins. One measurement:
drive SIG with a trigger pulse, switch the pin to input, and time the
echo pulse the module answers with; the pulse width divided by twice the
speed of sound is the distance.

The echo is timed via lgpio *alerts*, whose edge timestamps come from
the kernel rather than from Python — a Python polling loop would add
milliseconds of jitter, and one millisecond of pulse error is 17 cm of
distance error. (The trigger pulse only has a minimum width, so Python's
sleep granularity is harmless there.) There is no extension-hat option:
the Arduino-based hats are polled over I2C and cannot time the echo.

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import random
import socket
import sys
import threading
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Publish policy -----------------------------------------------------------

MEASURE_INTERVAL = 0.1
HEARTBEAT = 1.0
# Larger than the VL53L0X's delta because a sonar reading jitters a little.
MIN_DELTA_MM = 5

# -- Measurement --------------------------------------------------------------

# Sound travels 0.343 mm/µs = 343 mm per million ns; the echo pulse covers
# the distance twice.
MM_PER_NS = 0.343e-3 / 2.0

# The echo of a 3.5 m target takes ~20 ms; give up shortly after that.
ECHO_TIMEOUT_S = 0.06

MIN_RANGE_MM = 20
MAX_RANGE_MM = 3500


class UltrasonicSensor:
    """Times the ranger's single-wire trigger/echo cycle via lgpio."""

    def __init__(self, sig_pin=4, gpio_chip=0, name="ULTRASONIC"):
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
        self._pin = sig_pin
        self.name = name

        self._pulse_done = threading.Event()
        self._rise_ns = None
        self._pulse_ns = None
        # One persistent callback; it only fires while the pin is claimed
        # for alerts (i.e. during the echo phase of a measurement).
        self._callback = lgpio.callback(
            self._handle, self._pin, lgpio.BOTH_EDGES, self._on_edge
        )

    def _on_edge(self, chip, gpio, level, ticks):
        """Records the echo pulse from the kernel's edge timestamps [ns]."""
        if level == 1:
            self._rise_ns = ticks
        elif level == 0 and self._rise_ns is not None:
            self._pulse_ns = ticks - self._rise_ns
            self._pulse_done.set()

    def read_mm(self):
        """Return the distance in mm, or None without an echo in range."""
        lgpio = self._lgpio
        self._rise_ns = None
        self._pulse_ns = None
        self._pulse_done.clear()

        # Trigger: a >=10 µs high pulse. time.sleep overshoots, which the
        # module tolerates — the pulse only has a minimum width.
        lgpio.gpio_claim_output(self._handle, self._pin, 0)
        time.sleep(0.00002)
        lgpio.gpio_write(self._handle, self._pin, 1)
        time.sleep(0.00002)
        lgpio.gpio_write(self._handle, self._pin, 0)

        # Echo: hand the pin to the alert machinery and wait for the pulse.
        lgpio.gpio_claim_alert(self._handle, self._pin, lgpio.BOTH_EDGES)
        if not self._pulse_done.wait(ECHO_TIMEOUT_S):
            return None

        mm = round(self._pulse_ns * MM_PER_NS)
        if mm < MIN_RANGE_MM or mm > MAX_RANGE_MM:
            return None
        return mm

    def close(self):
        self._callback.cancel()
        self._lgpio.gpiochip_close(self._handle)


class EmulatedUltrasonicSensor(UltrasonicSensor):
    """Generates plausible ranger readings without hardware.

    A target sweeping back and forth between 200 and 2000 mm (20 s
    period), occasionally leaving the measuring range.
    """

    def __init__(self, name="ULTRASONIC"):
        self.name = name

    def read_mm(self):
        if random.random() < 0.02:
            return None
        phase = (time.time() % 20.0) / 20.0
        return round(200 + 1800 * (1 - abs(2 * phase - 1)))

    def close(self):
        pass


# -- WebSocket push server -----------------------------------------------------

_api_key = ""


async def measure_loop(sensor, publish):
    """Measure continuously; publish on change, range flip, or heartbeat."""
    loop = asyncio.get_event_loop()
    last_sent = None
    last_publish = 0.0
    ever_published = False

    while True:
        try:
            mm = sensor.read_mm()
        except OSError as e:
            print(f"GPIO read failed: {e}")
            await asyncio.sleep(MEASURE_INTERVAL)
            continue
        now = loop.time()
        changed = (
            not ever_published
            or (mm is None) != (last_sent is None)
            or (
                mm is not None
                and last_sent is not None
                and abs(mm - last_sent) >= MIN_DELTA_MM
            )
        )
        if changed or now - last_publish >= HEARTBEAT:
            await publish({"distance": mm})
            ever_published = True
            last_sent = mm
            last_publish = now
        await asyncio.sleep(MEASURE_INTERVAL)


async def main_async(sensor):
    server = wifi.WsPushServer(_api_key)
    async with server.serve():
        await measure_loop(sensor, server.broadcast)


async def main_ble(sensor, api_key):
    """Push readings over BLE instead of WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(sensor.name, api_key)
    await transport.start()
    try:
        await measure_loop(sensor, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    sig_pin = config.get("sig_pin", 4)
    gpio_chip = config.get("gpio_chip", 0)
    sensor_name = config.get("sensor_name", "ULTRASONIC")

    if config.get("emulation", False):
        print("Emulation mode: generating ranger readings without hardware")
        sensor = EmulatedUltrasonicSensor(name=sensor_name)
    else:
        print(f"Initializing ultrasonic ranger on GPIO {sig_pin}...")
        sensor = UltrasonicSensor(
            sig_pin=sig_pin, gpio_chip=gpio_chip, name=sensor_name
        )

    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(sensor, _api_key))
            return

        wifi.start_discovery_thread(sensor.name, hostname, wifi.WS_PORT)
        asyncio.run(main_async(sensor))
    finally:
        sensor.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
