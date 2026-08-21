"""
Sensor Playground Relay Node — Grove SPDT Relay, 1 / 2 / 4 channels (Python)

Implements the *actuator* variant of the Sensor Playground Sensor Interface on
single-board computers (Raspberry Pi & co.). Like the LED node it talks in
both directions: the app switches a channel and the node reports the resulting
state of every channel back.

How many channels the board carries comes from config.json (the length of
"relay_pins", or "channels" for the I2C module) and travels with the discovery
reply and with every state message, so the app draws exactly one button per
relay:

    app -> node   {"ch": 0, "on": true}      switch channel 0 (zero-based)
                  {"ch": 0, "toggle": true}  flip channel 0
                  {"all": false}             switch every channel off
    node -> app   {"channels": 2, "relay": [true, false]}
                                             state of the whole board (on
                                             connect and after every change)

The node owns the state; the app renders what the node last reported rather
than what it asked for, so a command that never arrived cannot leave the app
showing a closed contact that is in fact open.

The Grove SPDT relay modules come in two electrical flavours, selected by the
`interface` config value:

    "interface": "gpio"   1- and 2-channel modules on plain Pi GPIOs
                          (gpiozero; also Grove Base Hat digital ports, which
                          are wired straight to the Pi)
    "interface": "hat"    the same modules through an Arduino-based hat
                          (NanoHat Hub / GrovePi+) over I2C
    "interface": "i2c"    the 4-channel module, an I2C device whose on-board
                          MCU takes a channel bitmask (address 0x11)

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Over BLE the state arrives as a notify on the data characteristic and the
commands as short binary writes on the command characteristic (BLE writes are
binary-safe, so a switch costs three bytes instead of a JSON document):

    0x01 <ch> 0x00   switch channel <ch> off
    0x01 <ch> 0x01   switch channel <ch> on
    0x02 <ch>        flip channel <ch>
    0x03 0x00|0x01   switch every channel at once

Set "emulation": true in config.json to run without any relay hardware.

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

PUBLISH_INTERVAL = 0.02  # how often a pending state change reaches the wire

# -- BLE command framing -----------------------------------------------------

CMD_SET = 0x01     # 0x01 <channel> <0x00 off | 0x01 on>
CMD_TOGGLE = 0x02  # 0x02 <channel>
CMD_ALL = 0x03     # 0x03 <0x00 off | 0x01 on>

# -- Grove 4-Channel SPDT Relay (I2C) ----------------------------------------

# Command byte of the module's on-board MCU: takes a channel bitmask, bit 0
# being channel 1 (the same byte Seeed's Multi_Channel_Relay library sends).
I2C_CMD_CHANNEL_CTRL = 0x10


# -- Relay banks -------------------------------------------------------------


class GpioRelayBank:
    """Drives the 1-/2-channel modules from Raspberry Pi GPIOs (gpiozero)."""

    def __init__(self, pins, active_low):
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

        self.count = len(pins)
        try:
            # active_high is the polarity: with active_low wiring the coil is
            # energized when the pin is driven LOW, so .on() must drive LOW.
            self._devices = [
                DigitalOutputDevice(
                    pin, active_high=not active_low, initial_value=False
                )
                for pin in pins
            ]
        except Exception as e:
            # gpiozero is only a front-end and needs a pin factory backend. On
            # Raspberry Pi OS that is the system package python3-lgpio, which a
            # plain venv hides; gpiozero then falls back to its native factory,
            # which drives /sys/class/gpio — gone in Debian 13. The resulting
            # traceback points deep into gpiozero and says nothing about the
            # real cause, so translate it here.
            raise RuntimeError(
                f"Could not open the relay GPIOs {pins}: {e}\n"
                "If the traceback above mentions PinFactoryFallback or "
                "/sys/class/gpio, gpiozero found no pin factory backend. "
                "Recreate the venv with --system-site-packages so it can see "
                "the system python3-lgpio (an existing venv: set "
                "include-system-site-packages = true in venv/pyvenv.cfg). "
                "See this node's README."
            ) from e

    def write(self, states):
        """Drives every coil to the given per-channel states."""
        for device, on in zip(self._devices, states):
            device.value = 1 if on else 0

    def close(self):
        for device in self._devices:
            device.close()


class HatRelayBank:
    """Drives the 1-/2-channel modules through an Arduino-based hat."""

    def __init__(self, hat, pins, active_low):
        from extension_hat import PinMode

        self.count = len(pins)
        self._hat = hat
        self._pins = pins
        self._active_low = active_low
        for pin in pins:
            self._hat.pin_mode(pin, PinMode.OUTPUT)

    def write(self, states):
        """Drives every coil to the given per-channel states."""
        from extension_hat import DigitalValue

        for pin, on in zip(self._pins, states):
            high = (not on) if self._active_low else on
            self._hat.digital_write(
                pin, DigitalValue.HIGH if high else DigitalValue.LOW
            )

    def close(self):
        # The hat itself is closed by whoever created it (see make_relay_bank).
        pass


class I2cRelayBank:
    """Drives the Grove 4-Channel SPDT Relay over I2C.

    The module takes one bitmask for all channels, so every write pushes the
    whole board — there is nothing to read back and modify.
    """

    def __init__(self, i2c_bus, address, channels):
        from smbus2 import SMBus

        self.count = channels
        self._address = address
        self._bus = SMBus(i2c_bus)

    def write(self, states):
        """Drives every coil to the given per-channel states."""
        mask = 0
        for channel, on in enumerate(states):
            if on:
                mask |= 1 << channel
        self._bus.write_byte_data(self._address, I2C_CMD_CHANNEL_CTRL, mask)

    def close(self):
        self._bus.close()


class EmulatedRelayBank:
    """Prints the relay states instead of switching hardware."""

    def __init__(self, channels):
        self.count = channels

    def write(self, states):
        """Drives every coil to the given per-channel states."""
        shown = ", ".join(
            f"{index + 1}:{'on' if on else 'off'}"
            for index, on in enumerate(states)
        )
        print(f"[emulation] relays {shown}")

    def close(self):
        pass


# -- Relay state -------------------------------------------------------------


class RelayController:
    """Owns the channel states — the single source of truth this node
    publishes.

    Every switch marks the state as pending publication, even one that does
    not change it: a client that guessed wrong about the current state would
    otherwise never be corrected.
    """

    def __init__(self, bank):
        self._bank = bank
        self.states = [False] * bank.count
        self._pending = True  # publish the initial state as soon as we serve
        self._bank.write(self.states)

    @property
    def count(self):
        """Channels this board carries."""
        return len(self.states)

    def _valid(self, channel):
        if 0 <= channel < self.count:
            return True
        print(f"Ignoring command for channel {channel} "
              f"(board has {self.count})")
        return False

    def set(self, channel, on):
        """Switches one channel (zero-based) on or off."""
        if not self._valid(channel):
            return
        self.states[channel] = on
        self._bank.write(self.states)
        self._pending = True

    def toggle(self, channel):
        """Flips one channel (zero-based)."""
        if not self._valid(channel):
            return
        self.set(channel, not self.states[channel])

    def set_all(self, on):
        """Switches every channel of the board at once."""
        self.states = [on] * self.count
        self._bank.write(self.states)
        self._pending = True

    def payload(self):
        """The state message the app parses."""
        return {"channels": self.count, "relay": list(self.states)}

    def take_pending(self):
        """Returns True once after each switch, clearing the pending flag."""
        pending = self._pending
        self._pending = False
        return pending

    def close(self):
        self._bank.close()


def make_relay_bank(config):
    """Builds the configured relay bank.

    Returns (bank, hat) — [hat] is the shared extension hat when one is used
    and None otherwise; the caller closes it last.
    """
    interface = config.get("interface", "gpio")
    channels = config.get("channels", 4)
    pins = config.get("relay_pins")

    # The I2C module is told how wide it is; the wired ones are as wide as
    # their pin list, so that list is the channel count.
    if interface != "i2c":
        if not pins:
            raise ValueError(
                "No relay_pins configured — list one GPIO per channel, in "
                "channel order (or use \"interface\": \"i2c\" for the "
                "4-channel module)."
            )
        channels = len(pins)

    if config.get("emulation", False):
        # Without hardware the configuration still says how wide the board
        # is, so the app draws the same buttons as it would in the field.
        return EmulatedRelayBank(channels), None

    if interface == "i2c":
        i2c_bus = config.get("i2c_bus", 1)
        # Written as a hex string in config.json, like the other I2C nodes.
        address = int(str(config.get("i2c_address", "0x11")), 16)
        return I2cRelayBank(i2c_bus, address, channels), None

    active_low = config.get("relay_active_low", False)

    if interface == "gpio":
        return GpioRelayBank(pins, active_low), None

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
                f"Unsupported hat_type {hat_type!r} for a relay "
                "(use 'gpio' for a Grove Base Hat — its digital ports are "
                "wired straight to the Pi)."
            )
        hat.set_auto_wait(True)
        return HatRelayBank(hat, pins, active_low), hat

    raise ValueError(
        f"Unknown interface {interface!r} (use 'gpio', 'hat' or 'i2c')"
    )


# -- Commands ----------------------------------------------------------------


def handle_json_command(relay, message):
    """Executes one JSON command pushed by the app over the WebSocket."""
    try:
        command = json.loads(message)
    except json.JSONDecodeError:
        print("Ignoring malformed command")
        return

    if isinstance(command.get("all"), bool):
        on = command["all"]
        print(f"Command: all {'on' if on else 'off'}")
        relay.set_all(on)
        return

    channel = command.get("ch")
    if not isinstance(channel, int) or isinstance(channel, bool):
        print("Ignoring command without a channel")
        return

    if command.get("toggle") is True:
        print(f"Command: toggle channel {channel}")
        relay.toggle(channel)
        return

    on = command.get("on")
    if not isinstance(on, bool):
        print("Ignoring command without a boolean 'on'")
        return
    print(f"Command: channel {channel} {'on' if on else 'off'}")
    relay.set(channel, on)


def handle_ble_command(relay, packet):
    """Executes one binary command packet written over BLE."""
    if not packet:
        raise ValueError("empty command packet")

    opcode = packet[0]
    if opcode == CMD_ALL:
        if len(packet) < 2:
            raise ValueError("all without a state byte")
        on = packet[1] != 0
        print(f"Command: all {'on' if on else 'off'}")
        relay.set_all(on)
        return

    if opcode == CMD_TOGGLE:
        if len(packet) < 2:
            raise ValueError("toggle without a channel byte")
        print(f"Command: toggle channel {packet[1]}")
        relay.toggle(packet[1])
        return

    if opcode != CMD_SET:
        raise ValueError(f"unknown opcode {opcode:#04x}")
    if len(packet) < 3:
        raise ValueError("set without a channel and a state byte")

    on = packet[2] != 0
    print(f"Command: channel {packet[1]} {'on' if on else 'off'}")
    relay.set(packet[1], on)


# -- Serving -----------------------------------------------------------------

_api_key = ""


async def state_loop(relay, publish):
    """Publishes every state change of the board.

    Command handlers only mutate [relay] and this loop is what puts the result
    on the wire, so every change reaches the app the same way — command
    handlers run inside the transport's dispatch (a D-Bus callback, over BLE),
    where publishing directly would mean reaching across into the event loop.
    """
    while True:
        if relay.take_pending():
            print(f"relay: {relay.states}")
            await publish(relay.payload())
        await asyncio.sleep(PUBLISH_INTERVAL)


async def main_async(relay):
    """Serves the WebSocket transport."""

    async def send_current_state(websocket):
        await websocket.send(json.dumps(relay.payload()))

    server = wifi.WsPushServer(
        _api_key,
        on_connect=send_current_state,
        on_message=lambda message: handle_json_command(relay, message),
    )
    async with server.serve():
        await state_loop(relay, server.broadcast)


async def main_ble(relay, sensor_name, api_key):
    """Serves the BLE transport instead of the WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(
        sensor_name,
        api_key,
        on_command=lambda packet: handle_ble_command(relay, packet),
    )
    try:
        await transport.start()
        await state_loop(relay, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    sensor_name = config.get("sensor_name", "RELAY")

    if config.get("emulation", False):
        print("Emulation mode: switching virtual relays without hardware")
    else:
        print("Initializing relay node...")
    bank, hat = make_relay_bank(config)
    relay = RelayController(bank)
    print(f"Relay board: {relay.count} channel(s)")

    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(relay, sensor_name, _api_key))
            return

        # The app learns the board's width from the discovery reply as well,
        # so a scan already shows what it is about to open.
        wifi.start_discovery_thread(
            sensor_name,
            hostname,
            wifi.WS_PORT,
            lambda: {"channels": relay.count},
        )
        asyncio.run(main_async(relay))
    finally:
        # Leave every contact open rather than stuck closed after the node
        # exits.
        relay.set_all(False)
        relay.close()
        if hat is not None:
            hat.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
