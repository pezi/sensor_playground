"""
Sensor Tester Sensor Node — Digital Contact Sensor (Python, push)

One generic *push* node for the simple two-state Grove/BakeBit digital sensors
that react to an event:

    BUTTON     — Grove/BakeBit button      (pressed)
    HALL       — Grove Hall sensor         (magnetic field present)
    MAGSWITCH  — Grove magnetic switch     (reed switch closed)
    PIR        — Grove PIR motion sensor   (motion detected)
    VIBRATION  — Grove vibration sensor    (SW-420, vibration)

The node polls the input (debounced) and pushes one JSON message whenever the
state changes:

    {"active": true}    sensor triggered
    {"active": false}   sensor released

The original dart_periphery examples toggle a local LED; this node ignores
that and reports the state to the Sensor Tester app instead.

The input is read either through an Arduino-based extension hat (the sibling
`extension_hat` helper — NanoHat Hub / GrovePi+) or directly from a Raspberry
Pi GPIO pin (gpiozero), selected by the `interface` config value:

    "interface": "gpio"   read a Pi GPIO via gpiozero (also Grove Base Hat
                          digital ports, which are wired straight to the Pi)
    "interface": "hat"    read through an Arduino-based hat over I2C

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
import json
import socket
import sys
import time
from pathlib import Path


# The extension_hat helper lives in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "extension_hat"))
# The shared BLE transport lives in another sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

POLL_INTERVAL = 0.02   # 50 Hz input poll
DEBOUNCE_S = 0.03      # line must be stable this long before a change counts

# -- Inputs ------------------------------------------------------------------


class GpioInput:
    """Reads a digital input directly from a Raspberry Pi GPIO (gpiozero)."""

    def __init__(self, pin, active_low):
        from gpiozero import DigitalInputDevice

        # gpiozero's pull_up flag does two things at once, and active_low is
        # exactly the right value for both:
        #  - the internal pull resistor, so the idle state has a defined level
        #    (True -> pull-up, idle HIGH; False -> pull-down, idle LOW). This
        #    matters for reed-based sensors like the Grove Magnetic Switch,
        #    which only connect the line to VCC while the magnet holds them
        #    shut and float otherwise; without a pull-down the "released" state
        #    never reads a clean LOW.
        #  - the polarity: InputDevice sets active_state = (not pull_up), so
        #    .value is already 1 == "sensor triggered" rather than the raw pin
        #    level. read_active() must therefore NOT invert it again.
        try:
            self._device = DigitalInputDevice(pin, pull_up=active_low)
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

    def read_active(self):
        """Returns True while the sensor is triggered."""
        # Already polarity-corrected by pull_up=active_low — see __init__.
        return self._device.value == 1

    def close(self):
        self._device.close()


class HatInput:
    """Reads a digital input through an Arduino-based extension hat."""

    def __init__(self, hat_type, pin, active_low, i2c_bus):
        from extension_hat import (
            GrovePiPlusHat,
            NanoHatHub,
            PinMode,
        )

        if hat_type == "nano":
            self._hat = NanoHatHub(i2c_bus)
        elif hat_type == "grovePlus":
            self._hat = GrovePiPlusHat(i2c_bus)
        else:
            raise ValueError(
                f"Unsupported hat_type {hat_type!r} for a digital input "
                "(use 'gpio' for a Grove Base Hat — its digital ports are "
                "wired straight to the Pi)."
            )
        self._hat.set_auto_wait(True)
        # The Arduino-based hats expose only a plain INPUT: their AVR chip has
        # internal pull-ups but no internal pull-down (unlike the Pi GPIO
        # path). A sensor whose released state floats rather than being driven
        # — the reed-based Grove Magnetic Switch, which only connects the line
        # to VCC while the magnet holds it shut — therefore needs an *external*
        # pull-down resistor (e.g. 10 kOhm to GND) on the hat port; there is no
        # software pull to fall back on here.
        self._hat.pin_mode(pin, PinMode.INPUT)
        self._pin = pin
        self._active_low = active_low

    def read_active(self):
        """Returns True while the sensor is triggered."""
        from extension_hat import DigitalValue

        is_high = self._hat.digital_read(self._pin) == DigitalValue.HIGH
        return (not is_high) if self._active_low else is_high

    def close(self):
        self._hat.close()


class EmulatedDigitalContactSensor(GpioInput):
    """Generates plausible digital contact readings without hardware.

    The contact toggles roughly every five seconds, as if someone slowly
    pressed and released a button.
    """

    def __init__(self):
        pass

    def read_active(self):
        """Returns True while the sensor is triggered."""
        return int(time.time() / 5) % 2 == 0

    def close(self):
        pass


def make_input(config):
    """Builds the configured input source."""
    interface = config.get("interface", "gpio")
    pin = config["pin"]
    active_low = config.get("active_low", True)
    if interface == "gpio":
        return GpioInput(pin, active_low)
    if interface == "hat":
        return HatInput(
            config.get("hat_type", "nano"),
            pin,
            active_low,
            config.get("i2c_bus", 0),
        )
    raise ValueError(f"Unknown interface {interface!r} (use 'gpio' or 'hat')")


# -- WebSocket push server ----------------------------------------------------

_api_key = ""
_state = {"active": None}  # latest published state, shared with new clients


async def send_current_state(websocket):
    """Send the current state to a client right after it connects."""
    if _state["active"] is not None:
        await websocket.send(json.dumps({"active": _state["active"]}))


async def input_loop(sensor, publish):
    """Poll the input (debounced) and push each state change."""
    stable = None        # last published active state
    candidate = None     # pending state awaiting debounce
    candidate_since = 0.0
    loop = asyncio.get_running_loop()

    while True:
        active = sensor.read_active()
        now = loop.time()
        if active != candidate:
            candidate = active
            candidate_since = now
        elif active != stable and now - candidate_since >= DEBOUNCE_S:
            stable = active
            _state["active"] = active
            print(f"active: {active}")
            await publish({"active": active})
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(sensor):
    server = wifi.WsPushServer(_api_key, on_connect=send_current_state)
    async with server.serve():
        await input_loop(sensor, server.broadcast)


async def main_ble(sensor, sensor_name, api_key):
    """Push readings over BLE instead of WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(sensor_name, api_key)
    try:
        await transport.start()
        await input_loop(sensor, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    sensor_name = config.get("sensor_name", "BUTTON")

    if config.get("emulation", False):
        print(f"Emulation mode: generating {sensor_name} readings without hardware")
        sensor = EmulatedDigitalContactSensor()
    else:
        print(f"Initializing {sensor_name} digital contact sensor...")
        sensor = make_input(config)

    if config.get("transport", "wifi") == "ble":
        try:
            asyncio.run(main_ble(sensor, sensor_name, _api_key))
        finally:
            sensor.close()
        return

    wifi.start_discovery_thread(sensor_name, hostname, wifi.WS_PORT)

    try:
        asyncio.run(main_async(sensor))
    finally:
        sensor.close()


if __name__ == "__main__":
    main()
