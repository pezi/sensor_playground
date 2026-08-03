"""
Sensor Tester LED Node — LED + optional push button (Python)

Implements the *actuator* variant of the Sensor Tester Sensor Interface on
single-board computers (Raspberry Pi & co.). This is the only node that talks
in both directions: the app sends a switch command and the node reports the
resulting state back, because the LED can also be toggled by a push button
wired to the node itself.

    app -> node   {"led": true}     switch on
                  {"led": false}    switch off
                  {"toggle": true}  flip
    node -> app   {"led": true|false}   current state (on connect and after
                                        every change, whatever caused it)

The node owns the state; the app renders what the node last reported rather
than what it asked for, so a command that never arrived cannot leave the app
showing a lit LED that is in fact dark.

The LED and the button are driven either through an Arduino-based extension
hat (the sibling `extension_hat` helper — NanoHat Hub / GrovePi+) or directly
from Raspberry Pi GPIO pins (gpiozero), selected by the `interface` config
value:

    "interface": "gpio"   drive Pi GPIOs via gpiozero (also Grove Base Hat
                          digital ports, which are wired straight to the Pi)
    "interface": "hat"    drive them through an Arduino-based hat over I2C

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Over BLE the state arrives as a notify on the data characteristic and the
command as a short binary write on the command characteristic (BLE writes are
binary-safe, so a switch costs two bytes instead of a JSON document):

    0x01 0x00   switch off
    0x01 0x01   switch on
    0x02        flip

Set "button_pin": null in config.json for an LED-only node, and
"emulation": true to run without any hardware at all.

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import json
import socket
import sys
from pathlib import Path


# The extension_hat helper lives in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "extension_hat"))
# The shared transports (Wi-Fi + BLE) live in another sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

POLL_INTERVAL = 0.02   # 50 Hz button poll
DEBOUNCE_S = 0.03      # line must be stable this long before a press counts

# -- BLE command framing -----------------------------------------------------

CMD_SET = 0x01     # 0x01 <0x00 off | 0x01 on>
CMD_TOGGLE = 0x02  # flip


# -- Outputs -----------------------------------------------------------------


class GpioOutput:
    """Drives an LED directly from a Raspberry Pi GPIO (gpiozero)."""

    def __init__(self, pin, active_low):
        try:
            from gpiozero import DigitalOutputDevice
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
            # active_high is the polarity: with active_low wiring the LED
            # lights when the pin is driven LOW, so .on() must drive LOW.
            self._device = DigitalOutputDevice(
                pin, active_high=not active_low, initial_value=False
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

    def write(self, on):
        """Lights the LED when [on], darkens it otherwise."""
        self._device.value = 1 if on else 0

    def close(self):
        self._device.close()


class HatOutput:
    """Drives an LED through an Arduino-based extension hat."""

    def __init__(self, hat, pin, active_low):
        from extension_hat import PinMode

        self._hat = hat
        self._pin = pin
        self._active_low = active_low
        self._hat.pin_mode(pin, PinMode.OUTPUT)

    def write(self, on):
        """Lights the LED when [on], darkens it otherwise."""
        from extension_hat import DigitalValue

        high = (not on) if self._active_low else on
        self._hat.digital_write(
            self._pin, DigitalValue.HIGH if high else DigitalValue.LOW
        )

    def close(self):
        # The hat itself is closed by whoever created it (see make_hardware),
        # because the button input shares it.
        pass


class EmulatedOutput:
    """Prints the LED state instead of driving hardware."""

    def write(self, on):
        """Lights the LED when [on], darkens it otherwise."""
        print(f"[emulation] LED {'on' if on else 'off'}")

    def close(self):
        pass


# -- Inputs (optional local push button) --------------------------------------


class GpioButton:
    """Reads a push button directly from a Raspberry Pi GPIO (gpiozero)."""

    def __init__(self, pin, active_low):
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

        # gpiozero's pull_up flag does two things at once, and active_low is
        # exactly the right value for both: it picks the internal pull resistor
        # (True -> pull-up, idle HIGH) and the polarity, so .value is already
        # 1 == "pressed" rather than the raw pin level. read_pressed() must
        # therefore NOT invert it again.
        self._device = DigitalInputDevice(pin, pull_up=active_low)

    def read_pressed(self):
        """Returns True while the button is held down."""
        return self._device.value == 1

    def close(self):
        self._device.close()


class HatButton:
    """Reads a push button through an Arduino-based extension hat."""

    def __init__(self, hat, pin, active_low):
        from extension_hat import PinMode

        self._hat = hat
        self._pin = pin
        self._active_low = active_low
        self._hat.pin_mode(pin, PinMode.INPUT)

    def read_pressed(self):
        """Returns True while the button is held down."""
        from extension_hat import DigitalValue

        is_high = self._hat.digital_read(self._pin) == DigitalValue.HIGH
        return (not is_high) if self._active_low else is_high

    def close(self):
        # See HatOutput.close — the shared hat is closed in make_hardware's
        # caller.
        pass


class NoButton:
    """Stand-in for a node without a local button (or in emulation mode).

    Never reports a press, so the app is the only thing that switches the LED.
    """

    def read_pressed(self):
        """Returns True while the button is held down."""
        return False

    def close(self):
        pass


# -- LED state ---------------------------------------------------------------


class LedController:
    """Owns the LED state — the single source of truth this node publishes.

    Every switch marks the state as pending publication, even one that does
    not change it: a client that guessed wrong about the current state would
    otherwise never be corrected.
    """

    def __init__(self, output):
        self._output = output
        self.on = False
        self._pending = True  # publish the initial state as soon as we serve
        self._output.write(False)

    def set(self, on):
        """Switches the LED on or off."""
        self.on = on
        self._output.write(on)
        self._pending = True

    def toggle(self):
        """Flips the LED."""
        self.set(not self.on)

    def take_pending(self):
        """Returns True once after each switch, clearing the pending flag."""
        pending = self._pending
        self._pending = False
        return pending

    def close(self):
        self._output.close()


def make_hardware(config):
    """Builds the configured LED output and (optional) button input.

    Returns (output, button, hat) — [hat] is the shared extension hat when one
    is used and None otherwise; the caller closes it last.
    """
    if config.get("emulation", False):
        return EmulatedOutput(), NoButton(), None

    interface = config.get("interface", "gpio")
    led_pin = config["led_pin"]
    led_active_low = config.get("led_active_low", False)
    button_pin = config.get("button_pin")
    button_active_low = config.get("button_active_low", True)

    if interface == "gpio":
        output = GpioOutput(led_pin, led_active_low)
        button = (
            GpioButton(button_pin, button_active_low)
            if button_pin is not None
            else NoButton()
        )
        return output, button, None

    if interface == "hat":
        from extension_hat import GrovePiPlusHat, NanoHatHub

        hat_type = config.get("hat_type", "nano")
        i2c_bus = config.get("i2c_bus", 0)
        if hat_type == "nano":
            hat = NanoHatHub(i2c_bus)
        elif hat_type == "grovePlus":
            hat = GrovePiPlusHat(i2c_bus)
        else:
            raise ValueError(
                f"Unsupported hat_type {hat_type!r} for an LED "
                "(use 'gpio' for a Grove Base Hat — its digital ports are "
                "wired straight to the Pi)."
            )
        hat.set_auto_wait(True)
        output = HatOutput(hat, led_pin, led_active_low)
        button = (
            HatButton(hat, button_pin, button_active_low)
            if button_pin is not None
            else NoButton()
        )
        return output, button, hat

    raise ValueError(f"Unknown interface {interface!r} (use 'gpio' or 'hat')")


# -- Commands ----------------------------------------------------------------


def handle_json_command(led, message):
    """Executes one JSON command pushed by the app over the WebSocket."""
    try:
        command = json.loads(message)
    except json.JSONDecodeError:
        print("Ignoring malformed command")
        return

    if command.get("toggle") is True:
        print("Command: toggle")
        led.toggle()
        return

    on = command.get("led")
    if not isinstance(on, bool):
        print("Ignoring command without a boolean 'led'")
        return
    print(f"Command: led {'on' if on else 'off'}")
    led.set(on)


def handle_ble_command(led, packet):
    """Executes one binary command packet written over BLE."""
    if not packet:
        raise ValueError("empty command packet")

    opcode = packet[0]
    if opcode == CMD_TOGGLE:
        print("Command: toggle")
        led.toggle()
        return
    if opcode != CMD_SET:
        raise ValueError(f"unknown opcode {opcode:#04x}")
    if len(packet) < 2:
        raise ValueError("set without a state byte")

    on = packet[1] != 0
    print(f"Command: led {'on' if on else 'off'}")
    led.set(on)


# -- Serving -----------------------------------------------------------------

_api_key = ""


async def state_loop(led, button, publish):
    """Polls the button (debounced) and publishes every state change.

    Both sources of change funnel through here — a command handler only
    mutates [led] and this loop is what puts the result on the wire, so the
    app sees an app-initiated switch and a button press the same way. Command
    handlers run inside the transport's dispatch (a D-Bus callback, over BLE),
    where publishing directly would mean reaching across into the event loop.
    """
    pressed = None       # last debounced button state
    candidate = None     # pending state awaiting debounce
    candidate_since = 0.0
    loop = asyncio.get_running_loop()

    while True:
        now = loop.time()
        is_pressed = button.read_pressed()
        if is_pressed != candidate:
            candidate = is_pressed
            candidate_since = now
        elif is_pressed != pressed and now - candidate_since >= DEBOUNCE_S:
            first_reading = pressed is None
            pressed = is_pressed
            # The first stable reading only establishes the idle level;
            # treating it as an edge would toggle the LED at startup whenever
            # the button happens to be held. Toggle on press, not on release,
            # so one press is one toggle.
            if is_pressed and not first_reading:
                print("Button pressed: toggling LED")
                led.toggle()

        if led.take_pending():
            print(f"led: {led.on}")
            await publish({"led": led.on})
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(led, button):
    """Serves the WebSocket transport."""

    async def send_current_state(websocket):
        await websocket.send(json.dumps({"led": led.on}))

    server = wifi.WsPushServer(
        _api_key,
        on_connect=send_current_state,
        on_message=lambda message: handle_json_command(led, message),
    )
    async with server.serve():
        await state_loop(led, button, server.broadcast)


async def main_ble(led, button, sensor_name, api_key):
    """Serves the BLE transport instead of the WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(
        sensor_name,
        api_key,
        on_command=lambda packet: handle_ble_command(led, packet),
    )
    try:
        await transport.start()
        await state_loop(led, button, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    sensor_name = config.get("sensor_name", "LED")

    if config.get("emulation", False):
        print("Emulation mode: switching a virtual LED without hardware")
    else:
        print("Initializing LED node...")
    output, button, hat = make_hardware(config)
    led = LedController(output)

    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(led, button, sensor_name, _api_key))
            return

        wifi.start_discovery_thread(sensor_name, hostname, wifi.WS_PORT)
        asyncio.run(main_async(led, button))
    finally:
        # Leave the LED dark rather than stuck on after the node exits.
        led.set(False)
        led.close()
        button.close()
        if hat is not None:
            hat.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
