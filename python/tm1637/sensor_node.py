"""
Sensor Tester Clock Node — Grove 4-Digit Display / TM1637 (Python)

Implements the *actuator* variant of the Sensor Tester Sensor Interface on
single-board computers (Raspberry Pi & co.). Like the LED node it talks in
both directions: the app pushes the time to show (and a brightness), and the
node reports the state it is actually displaying — which keeps changing on
its own, because once a time is set the node advances the minute and blinks
the colon autonomously.

    app -> node   {"time": "HH:MM"}     set the displayed time (24-hour)
    node -> app   {"time": "12:34", "brightness": 3}   current state
                  {"time": null, "brightness": 3}      no time set yet

The node pushes its state on connect, after every accepted command, and on
each minute rollover — never on the colon blink, so state traffic stays at
one message a minute. Before the first time set the display shows "--:--".

The node owns the state; the app renders what the node last reported rather
than what it asked for, so a command that never arrived cannot leave the app
showing a time the display does not.

The TM1637 speaks a proprietary two-wire protocol (start/stop conditions like
I2C, but LSB-first and without addresses) and has no minimum clock speed, so
it is bit-banged directly on two Pi GPIOs via gpiozero:

    "interface": "gpio"   drive Pi GPIOs via gpiozero (also Grove Base Hat
                          digital ports, which are wired straight to the Pi)

The Arduino-based extension hats ("interface": "hat") are NOT supported for
this node: one frame needs ~50 line transitions and each hat digital_write is
a full I2C transaction, far too slow for a display bus.

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Over BLE the state arrives as a notify on the data characteristic and the
command as a short binary write on the command characteristic:

    0x01 <hh> <mm>   set the time (rejected unless hh<=23 and mm<=59)
    0x02 <0..7>      set the brightness (clamped)
    0x03             re-notify the current state

Set "emulation": true to run without any hardware at all.

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import json
import re
import socket
import sys
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

POLL_INTERVAL = 0.05       # 20 Hz clock/blink tick
COLON_BLINK_S = 0.5        # colon on for 500 ms, off for 500 ms
DEFAULT_BRIGHTNESS = 3

# -- BLE command framing -----------------------------------------------------

CMD_SET_TIME = 0x01       # 0x01 <hh> <mm>
CMD_BRIGHTNESS = 0x02     # 0x02 <0..7>
CMD_STATE_REQUEST = 0x03  # re-notify the current state

# Segment patterns for the digits 0-9, gfedcba bit order.
SEGMENT_DIGITS = (0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F)
# A lone middle segment (g), the "no time yet" placeholder digit.
SEGMENT_DASH = 0x40
# The Grove module wires the colon to bit 7 of the second digit.
SEGMENT_COLON = 0x80


# -- Displays ----------------------------------------------------------------


class GpioDisplay:
    """Bit-bangs a TM1637 4-digit display on two Pi GPIOs (gpiozero).

    The chip has no minimum clock speed, so plain Python pin toggling is
    fast enough — a frame takes a few milliseconds.
    """

    def __init__(self, clk_pin, dio_pin):
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
            self._clk = DigitalOutputDevice(clk_pin, initial_value=False)
            self._dio = DigitalOutputDevice(dio_pin, initial_value=False)
        except Exception as e:
            # gpiozero is only a front-end and needs a pin factory backend. On
            # Raspberry Pi OS that is the system package python3-lgpio, which a
            # plain venv hides; gpiozero then falls back to its native factory,
            # which drives /sys/class/gpio — gone in Debian 13. The resulting
            # traceback points deep into gpiozero and says nothing about the
            # real cause, so translate it here.
            raise RuntimeError(
                f"Could not open GPIOs {clk_pin}/{dio_pin}: {e}\n"
                "If the traceback above mentions PinFactoryFallback or "
                "/sys/class/gpio, gpiozero found no pin factory backend. "
                "Recreate the venv with --system-site-packages so it can see "
                "the system python3-lgpio (an existing venv: set "
                "include-system-site-packages = true in venv/pyvenv.cfg). "
                "See this node's README."
            ) from e

    def _start(self):
        # DIO falls while CLK is high.
        self._clk.on()
        self._dio.on()
        self._dio.off()
        self._clk.off()

    def _stop(self):
        # DIO rises while CLK is high.
        self._clk.off()
        self._dio.off()
        self._clk.on()
        self._dio.on()

    def _write_byte(self, value):
        for bit in range(8):
            self._clk.off()
            self._dio.value = (value >> bit) & 1
            self._clk.on()
        # ACK slot: the chip acknowledges by pulling DIO low itself. An
        # output-only gpiozero pin cannot be released to read it, so drive
        # DIO low through the ninth clock pulse — the same level the chip is
        # asserting, so nothing conflicts and there is nothing useful to do
        # on a NAK anyway.
        self._clk.off()
        self._dio.off()
        self._clk.on()
        self._clk.off()

    def render(self, hour, minute, colon_on, brightness):
        """Writes the time (or dashes when [hour] is None) to the panel."""
        if hour is None or minute is None:
            segments = [SEGMENT_DASH] * 4
        else:
            segments = [
                SEGMENT_DIGITS[hour // 10],
                SEGMENT_DIGITS[hour % 10],
                SEGMENT_DIGITS[minute // 10],
                SEGMENT_DIGITS[minute % 10],
            ]
            if colon_on:
                segments[1] |= SEGMENT_COLON

        self._start()
        self._write_byte(0x40)  # data command: write, auto-increment address
        self._stop()

        self._start()
        self._write_byte(0xC0)  # address command: start at digit 0
        for segment in segments:
            self._write_byte(segment)
        self._stop()

        self._start()
        self._write_byte(0x88 | (brightness & 0x07))  # display on
        self._stop()

    def close(self):
        # Blank the panel rather than leaving a stale time burning.
        self.render(None, None, False, 0)
        self._clk.close()
        self._dio.close()


class EmulatedDisplay:
    """Prints the displayed state instead of driving hardware.

    Prints on time/brightness changes and minute rollovers only — the colon
    blink would flood the console at 1 Hz.
    """

    def __init__(self):
        self._last_printed = None

    def render(self, hour, minute, colon_on, brightness):
        """Writes the time (or dashes when [hour] is None) to the panel."""
        shown = (hour, minute, brightness)
        if shown == self._last_printed:
            return
        self._last_printed = shown
        text = "--:--" if hour is None else f"{hour:02d}:{minute:02d}"
        print(f"[emulation] display [{text}] brightness={brightness}")

    def close(self):
        pass


# -- Clock state --------------------------------------------------------------


class ClockController:
    """Owns the displayed clock state — the single source of truth this node
    publishes — and keeps it ticking.

    Every command marks the state as pending publication, even one that does
    not change it: a client that guessed wrong about the current state would
    otherwise never be corrected. The colon blink renders but never marks
    pending; the minute rollover does both.
    """

    def __init__(self, display, brightness):
        self._display = display
        self.hour = None
        self.minute = None
        self.brightness = max(0, min(7, brightness))
        self._pending = True  # publish the initial state as soon as we serve
        self._colon_on = False
        self._last_blink = 0.0
        self._minute_accum = 0.0
        self._last_tick = None
        self._render()

    def _render(self):
        self._display.render(
            self.hour, self.minute, self._colon_on, self.brightness
        )

    def set_time(self, hour, minute):
        """Sets the displayed time (24-hour) and restarts the minute phase."""
        self.hour = hour
        self.minute = minute
        # ":00 seconds" is now, and a lit colon gives immediate feedback.
        # time.monotonic() rather than the asyncio clock, because command
        # handlers may run outside the event loop (BLE D-Bus dispatch).
        self._minute_accum = 0.0
        self._colon_on = True
        self._last_blink = time.monotonic()
        self._render()
        self._pending = True

    def set_brightness(self, brightness):
        """Sets the display brightness, clamped to 0..7."""
        self.brightness = max(0, min(7, int(brightness)))
        self._render()
        self._pending = True

    def request_state(self):
        """Marks the state for re-publication without changing it."""
        self._pending = True

    def tick(self):
        """Advances the local clock and blinks the colon.

        Only the minute rollover marks the state pending; the blink is
        render-only.
        """
        now = time.monotonic()
        if self._last_tick is None:
            self._last_tick = now
        elapsed = now - self._last_tick
        self._last_tick = now

        if self.hour is None or self.minute is None:
            return

        if now - self._last_blink >= COLON_BLINK_S:
            self._last_blink = now
            self._colon_on = not self._colon_on
            self._render()

        self._minute_accum += elapsed
        if self._minute_accum < 60.0:
            return
        self._minute_accum -= 60.0
        self.minute += 1
        if self.minute >= 60:
            self.minute = 0
            self.hour += 1
            if self.hour >= 24:  # midnight rollover
                self.hour = 0
        self._render()
        self._pending = True

    def take_pending(self):
        """Returns True once after each change, clearing the pending flag."""
        pending = self._pending
        self._pending = False
        return pending

    def state(self):
        """Returns the state payload dict the transports publish."""
        time_text = (
            None
            if self.hour is None or self.minute is None
            else f"{self.hour:02d}:{self.minute:02d}"
        )
        return {"time": time_text, "brightness": self.brightness}

    def close(self):
        self._display.close()


def make_display(config):
    """Builds the configured display driver."""
    if config.get("emulation", False):
        return EmulatedDisplay()

    interface = config.get("interface", "gpio")
    if interface == "gpio":
        return GpioDisplay(config["clk_pin"], config["dio_pin"])

    if interface == "hat":
        raise ValueError(
            "The Arduino-based extension hats cannot drive a TM1637: one "
            "frame needs ~50 line transitions and each hat digital_write is "
            "a full I2C transaction. Wire the display to Pi GPIOs and use "
            "'gpio' (a Grove Base Hat's digital ports work — they are wired "
            "straight to the Pi)."
        )

    raise ValueError(f"Unknown interface {interface!r} (use 'gpio')")


# -- Commands ----------------------------------------------------------------

_TIME_PATTERN = re.compile(r"^(\d{2}):(\d{2})$")


def parse_time_text(text):
    """Parses an "HH:MM" string, returning (hour, minute) or None."""
    if not isinstance(text, str):
        return None
    match = _TIME_PATTERN.match(text)
    if match is None:
        return None
    hour, minute = int(match.group(1)), int(match.group(2))
    if hour > 23 or minute > 59:
        return None
    return hour, minute


def handle_json_command(clock, message):
    """Executes one JSON command pushed by the app over the WebSocket."""
    try:
        command = json.loads(message)
    except json.JSONDecodeError:
        print("Ignoring malformed command")
        return

    if "time" in command:
        parsed = parse_time_text(command.get("time"))
        if parsed is None:
            print("Ignoring invalid time")
            return
        hour, minute = parsed
        print(f"Command: time {hour:02d}:{minute:02d}")
        clock.set_time(hour, minute)
        return

    brightness = command.get("brightness")
    if isinstance(brightness, bool) or not isinstance(brightness, int):
        print("Ignoring unknown command")
        return
    print(f"Command: brightness {brightness}")
    clock.set_brightness(brightness)


def handle_ble_command(clock, packet):
    """Executes one binary command packet written over BLE."""
    if not packet:
        raise ValueError("empty command packet")

    opcode = packet[0]
    if opcode == CMD_SET_TIME:
        if len(packet) < 3 or packet[1] > 23 or packet[2] > 59:
            raise ValueError("invalid time command")
        print(f"Command: time {packet[1]:02d}:{packet[2]:02d}")
        clock.set_time(packet[1], packet[2])
        return
    if opcode == CMD_BRIGHTNESS:
        if len(packet) < 2:
            raise ValueError("brightness without a value byte")
        print(f"Command: brightness {packet[1]}")
        clock.set_brightness(packet[1])
        return
    if opcode == CMD_STATE_REQUEST:
        print("Command: state request")
        clock.request_state()
        return
    raise ValueError(f"unknown opcode {opcode:#04x}")


# -- Serving -----------------------------------------------------------------

_api_key = ""


async def state_loop(clock, publish):
    """Ticks the clock and publishes every pending state.

    All sources of change funnel through here — a command handler only
    mutates [clock] and this loop is what puts the result on the wire, so the
    app sees a command echo and a minute rollover the same way. Command
    handlers run inside the transport's dispatch (a D-Bus callback, over
    BLE), where publishing directly would mean reaching across into the
    event loop.
    """
    while True:
        clock.tick()
        if clock.take_pending():
            state = clock.state()
            print(f"state: {state}")
            await publish(state)
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(clock):
    """Serves the WebSocket transport."""

    async def send_current_state(websocket):
        await websocket.send(json.dumps(clock.state()))

    server = wifi.WsPushServer(
        _api_key,
        on_connect=send_current_state,
        on_message=lambda message: handle_json_command(clock, message),
    )
    async with server.serve():
        await state_loop(clock, server.broadcast)


async def main_ble(clock, sensor_name, api_key):
    """Serves the BLE transport instead of the WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(
        sensor_name,
        api_key,
        on_command=lambda packet: handle_ble_command(clock, packet),
    )
    try:
        await transport.start()
        await state_loop(clock, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    sensor_name = config.get("sensor_name", "TM1637")

    if config.get("emulation", False):
        print("Emulation mode: driving a virtual display without hardware")
    else:
        print("Initializing TM1637 clock node...")
    display = make_display(config)
    clock = ClockController(display, config.get("brightness", DEFAULT_BRIGHTNESS))

    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(clock, sensor_name, _api_key))
            return

        wifi.start_discovery_thread(sensor_name, hostname, wifi.WS_PORT)
        asyncio.run(main_async(clock))
    finally:
        clock.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
