"""
Sensor Tester Sensor Node — Grove Dust Sensor / Shinyei PPD42NS (Python)

Implements the Sensor Tester Sensor Interface on single-board computers with a
Grove Dust Sensor (Shinyei PPD42NS). The sensor pulls its output pin LOW while
particles scatter light inside its chamber; the node accumulates that low-pulse
occupancy (LPO) over 30-second windows and converts the ratio into a particle
concentration in pcs/0.01cf using the Nafis curve, reported under the JSON key
`dust`:

    ratio         = low_time / window_time * 100          (percent)
    concentration = 1.1*r^3 - 3.8*r^2 + 520*r + 0.62      (pcs/0.01cf)

https://wiki.seeedstudio.com/Grove-Dust_Sensor/
https://www.howmuchsnow.com/arduino/airquality/grovedust/

The pin is read directly via gpiozero edge callbacks — there is no extension
hat option: the Arduino-based hats are polled over I2C and cannot timestamp
the sensor's 10-90 ms pulses.

The first reading appears after the first full 30-second window; until then
the REST endpoint answers 503 ("Sensor read failed") and the discovery reply
carries no `dust` value. That is warm-up, not an error.

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
from pathlib import Path


# The shared transports live in the sibling common folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Sensor ------------------------------------------------------------------


class Ppd42nsSensor:
    """Accumulates PPD42NS low-pulse occupancy into a dust concentration."""

    WINDOW_S = 30.0

    def __init__(self, pin):
        try:
            from gpiozero import DigitalInputDevice
        except ImportError as e:
            # A raw ModuleNotFoundError here usually means the node runs
            # outside its venv, or the venv was never populated.
            raise RuntimeError(
                "gpiozero is not installed. Activate this node's venv and "
                "run 'pip install -r requirements.txt' — the venv must be "
                "created with --system-site-packages (see README, "
                "\"Setup\")."
            ) from e

        try:
            # The PPD42NS drives its output actively (through the divider), so
            # no internal pull is needed; active_state=False makes "activated"
            # mean "pin LOW" — i.e. a dust pulse in progress.
            self._device = DigitalInputDevice(
                pin, pull_up=None, active_state=False
            )
        except Exception as e:
            # gpiozero is only a front-end and needs a pin factory backend. On
            # Raspberry Pi OS that is the system package python3-lgpio, which a
            # plain venv hides; gpiozero then falls back to its native factory,
            # which drives /sys/class/gpio — gone in Debian 13. The resulting
            # traceback points deep into gpiozero and says nothing about the
            # real cause, so translate it here.
            raise RuntimeError(
                f"Could not open GPIO {pin}: {e}\n"
                "If the traceback above mentions PinFactoryFallback or "
                "/sys/class/gpio, gpiozero found no pin factory backend. "
                "Recreate the venv with --system-site-packages so it can see "
                "the system python3-lgpio (an existing venv: set "
                "include-system-site-packages = true in venv/pyvenv.cfg). "
                "See this node's README."
            ) from e

        self._lock = threading.Lock()
        self._lpo = 0.0            # seconds of low time this window
        self._low_since = None     # monotonic time of the current falling edge
        self._window_start = time.monotonic()
        self._concentration = None  # None until the first window closes
        self._device.when_activated = self._on_low
        self._device.when_deactivated = self._on_high
        # Edge callbacks never fire for a pin that is already low (a stuck or
        # misbehaving line), which would read as 0 % occupancy — "perfectly
        # clean air" — instead of saturation. Treat an initial low as a pulse
        # in progress so a stuck-low line reports a huge value, not a clean one.
        if self._device.is_active:
            self._low_since = time.monotonic()
        self.name = "PPD42NS"

    def _on_low(self):
        with self._lock:
            self._low_since = time.monotonic()

    def _on_high(self):
        with self._lock:
            if self._low_since is not None:
                self._lpo += time.monotonic() - self._low_since
                self._low_since = None

    def _maybe_roll(self):
        """Closes the LPO window once it is due and caches the result.

        Called lazily from read(), so the window grows past WINDOW_S when
        nobody polls — harmless, because the ratio divides by the *actual*
        elapsed time rather than the nominal window length.
        """
        now = time.monotonic()
        with self._lock:
            elapsed = now - self._window_start
            if elapsed < self.WINDOW_S:
                return
            lpo = self._lpo
            if self._low_since is not None:
                # A pulse spans the boundary: credit the elapsed part to the
                # closing window and restart the timer for the new one.
                lpo += now - self._low_since
                self._low_since = now
            self._lpo = 0.0
            self._window_start = now
        ratio = lpo / elapsed * 100.0
        self._concentration = (
            1.1 * ratio**3 - 3.8 * ratio**2 + 520.0 * ratio + 0.62
        )

    def read(self):
        """Return full-key readings for the REST API (None while warming up)."""
        self._maybe_roll()
        if self._concentration is None:
            return None
        return {"dust": round(self._concentration, 1)}

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        # The dust value uses the same key on both payloads. Must return a
        # dict even during warm-up: the discovery listener merges it into the
        # reply and would drop the whole reply on a None.
        return self.read() or {}

    def close(self):
        self._device.close()


class EmulatedPpd42nsSensor(Ppd42nsSensor):
    """Generates plausible PPD42NS readings without hardware.

    Indoor air over the day: the concentration follows a slow sine between
    roughly 100 and 700 pcs/0.01cf — deliberately crossing several Dylos
    air-quality bands — with a little measurement jitter, never below zero.
    """

    def __init__(self):
        self.name = "PPD42NS"

    def read(self):
        t = time.time()
        concentration = 400 + 300 * math.sin(t / 180.0) + random.uniform(-40, 40)
        return {"dust": round(max(0.0, concentration), 1)}

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
    pin = config.get("pin", 17)
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating PPD42NS readings without hardware")
        _sensor = EmulatedPpd42nsSensor()
    else:
        print(f"Initializing Grove Dust Sensor (PPD42NS) on GPIO {pin}...")
        print("First reading after the first full 30-second LPO window.")
        _sensor = Ppd42nsSensor(pin)

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
        _sensor.name,
        _api_key,
        _hostname,
        _sensor.read,
        ssl_cert,
        ssl_key,
        empty_error="warming_up",
    )


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
