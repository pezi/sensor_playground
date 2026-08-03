"""
Sensor Tester Sensor Node — Grove Rotary Angle Sensor (Python, push)

Reads a Grove Rotary Angle Sensor — a 10 kOhm potentiometer with 300° of
mechanical travel — through an extension hat's ADC and reports the knob
position.
https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/

Unlike the light sensor (also analog, but polled over HTTPS every few
seconds) this is a *push* node: the app draws a needle that tracks the knob,
so a reading that is seconds old is useless. The node samples the ADC
continuously and sends a message whenever the knob has moved further than the
deadband, plus the current position once per client connect:

    {"adc": 2048, "adcMax": 4095, "angle": 150.1, "angleMax": 300.0}

ADC RANGE — why adcMax is in every message
------------------------------------------
The converter's width belongs to the board doing the reading, not to the
knob, and the boards this project supports do not agree:

    "hat_type": "grove"       Seeed Grove Base Hat      12-bit   0 - 4095
    "hat_type": "nano"        FriendlyARM NanoHat Hub   10-bit   0 - 1023
    "hat_type": "grovePlus"   Seeed GrovePi+            10-bit   0 - 1023
    (the ESP32 sketch)                                  12-bit   0 - 4095

A bare "512" is therefore ambiguous — half travel on a NanoHat, an eighth on
a Grove Base Hat. Sending the full-scale value alongside the count lets the
app scale the dial to whichever node it is talking to instead of assuming one
of them. `angle`/`angleMax` carry the same position expressed in degrees, so
the app never needs to know the knob's mechanical travel either.

The Raspberry Pi has no analog input, so an analog sensor needs an extension
hat with an ADC — there is no direct-GPIO option (unlike the digital contact
node).

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
import math
import socket
import sys
import time
from pathlib import Path


# The extension_hat helper lives in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "extension_hat"))
# The shared transports (Wi-Fi + BLE) live in another sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

POLL_INTERVAL = 0.04  # 25 Hz — matches the ESP32 sketch's publish ceiling

# Full-scale ADC count per hat, keyed by hat_type. This is the number the app
# scales its dial by, so it has to match what the hat's read actually returns.
ADC_MAX_BY_HAT = {
    "grove": 4095,      # Grove Base Hat, 12-bit STM32F030 ADC
    "nano": 1023,       # NanoHat Hub, 10-bit AVR analogRead (BakeBit)
    "grovePlus": 1023,  # GrovePi+, 10-bit ATMEGA328P analogRead
}

# Mechanical travel of the knob, end to end. 300° for the Grove sensor.
DEFAULT_ANGLE_MAX = 300.0

# Default deadband as a fraction of full scale: how far the count must move
# before a new message goes out. ADC noise jitters the reading by a few counts
# with the knob at rest, which would otherwise flood the link.
DEFAULT_DEADBAND_RATIO = 0.006  # ~24 counts of 4095, ~6 of 1023


# -- Sensor ------------------------------------------------------------------


class RotaryAngleSensor:
    """Reads a Grove Rotary Angle Sensor through an extension hat's ADC."""

    def __init__(self, hat_type="grove", pin=0, i2c_bus=1, angle_max=None):
        from extension_hat import GroveBaseHat, GrovePiPlusHat, NanoHatHub

        if hat_type not in ADC_MAX_BY_HAT:
            raise ValueError(
                f"Unknown hat_type {hat_type!r} "
                f"(use one of {', '.join(sorted(ADC_MAX_BY_HAT))})"
            )

        if hat_type == "grove":
            self._hat = GroveBaseHat(i2c_bus)
            self._read = lambda: self._hat.read_adc_raw(pin)
        elif hat_type == "nano":
            self._hat = NanoHatHub(i2c_bus)
            self._read = lambda: self._hat.analog_read(pin)
        else:
            self._hat = GrovePiPlusHat(i2c_bus)
            self._read = lambda: self._hat.analog_read(pin)

        self.name = "ROTARY"
        self.adc_max = ADC_MAX_BY_HAT[hat_type]
        self.angle_max = angle_max if angle_max is not None else DEFAULT_ANGLE_MAX

    def read_adc(self):
        """Returns the raw count, clamped to 0..adc_max.

        The Arduino-based hats return -1 for a failed read; clamping turns
        that into a 0 rather than letting a negative angle reach the app.
        """
        return max(0, min(self.adc_max, self._read()))

    def close(self):
        self._hat.close()


class EmulatedRotaryAngleSensor(RotaryAngleSensor):
    """Generates plausible knob movements without hardware.

    Sweeps slowly from end to end and back, as if someone were turning the
    knob by hand, so the app's needle has something to follow.
    """

    def __init__(self, hat_type="grove", angle_max=None):
        self.name = "ROTARY"
        # Emulate the range the configured hat would really report, so the
        # app is exercised against a 10-bit node as easily as a 12-bit one.
        self.adc_max = ADC_MAX_BY_HAT.get(hat_type, 4095)
        self.angle_max = angle_max if angle_max is not None else DEFAULT_ANGLE_MAX

    def read_adc(self):
        """Returns the raw count, clamped to 0..adc_max."""
        # A 20-second triangle wave over the full span.
        phase = (time.time() % 20.0) / 20.0
        fraction = 1 - abs(2 * phase - 1)
        return round(fraction * self.adc_max)

    def close(self):
        pass


def payload_for(sensor, adc):
    """Builds the position message. The scale travels with every reading."""
    return {
        "adc": adc,
        "adcMax": sensor.adc_max,
        "angle": round(adc / sensor.adc_max * sensor.angle_max, 1),
        "angleMax": sensor.angle_max,
    }


# -- Serving -----------------------------------------------------------------

_api_key = ""
_sensor = None
_last_adc = None  # latest published count, shared with newly connected clients


async def send_current_state(websocket):
    """Sends the current position to a client right after it connects."""
    adc = _last_adc if _last_adc is not None else _sensor.read_adc()
    await websocket.send(json.dumps(payload_for(_sensor, adc)))


async def rotary_loop(sensor, publish, deadband):
    """Samples the knob and publishes whenever it moves past [deadband]."""
    global _last_adc
    published = None

    while True:
        adc = sensor.read_adc()
        moved = published is None or abs(adc - published) >= deadband
        # Pin the ends too, so a knob turned fully reports exactly 0 or full
        # scale instead of stopping a deadband short of it.
        at_end = adc in (0, sensor.adc_max) and adc != published
        if moved or at_end:
            published = adc
            _last_adc = adc
            payload = payload_for(sensor, adc)
            print(f"adc: {adc}/{sensor.adc_max}  angle: {payload['angle']}")
            await publish(payload)
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(sensor, deadband):
    server = wifi.WsPushServer(_api_key, on_connect=send_current_state)
    async with server.serve():
        await rotary_loop(sensor, server.broadcast, deadband)


async def main_ble(sensor, api_key, deadband):
    """Pushes readings over BLE instead of WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(sensor.name, api_key)
    try:
        await transport.start()
        await rotary_loop(sensor, transport.publish, deadband)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key, _sensor

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    hat_type = config.get("hat_type", "grove")
    pin = config.get("pin", 0)
    i2c_bus = config.get("i2c_bus", 1)
    angle_max = config.get("angle_max", DEFAULT_ANGLE_MAX)

    if config.get("emulation", False):
        print("Emulation mode: generating ROTARY readings without hardware")
        _sensor = EmulatedRotaryAngleSensor(hat_type=hat_type, angle_max=angle_max)
    else:
        print(
            f"Initializing Grove Rotary Angle Sensor on {hat_type} hat, "
            f"channel {pin}..."
        )
        _sensor = RotaryAngleSensor(
            hat_type=hat_type, pin=pin, i2c_bus=i2c_bus, angle_max=angle_max
        )

    print(f"ADC range: 0-{_sensor.adc_max}, travel {_sensor.angle_max:.0f} deg")

    # A deadband given in counts wins; otherwise scale it to this hat's range
    # so a 10-bit node is not held to a 12-bit node's precision.
    deadband = config.get("deadband")
    if deadband is None:
        deadband = max(1, math.ceil(_sensor.adc_max * DEFAULT_DEADBAND_RATIO))

    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(_sensor, _api_key, deadband))
            return

        wifi.start_discovery_thread(_sensor.name, hostname, wifi.WS_PORT)
        asyncio.run(main_async(_sensor, deadband))
    finally:
        _sensor.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
