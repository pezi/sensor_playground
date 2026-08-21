"""
Sensor Playground Speaker Node — Grove Speaker (Python)

Implements the *actuator* variant of the Sensor Playground Sensor Interface on
single-board computers (Raspberry Pi & co.) with a Grove Speaker — a small
amplified loudspeaker on a digital pin, driven with a square wave of the
desired pitch (PWM at 50% duty via lgpio). Like the LED the node talks in
both directions: the app asks for a tone, the built-in melody, or silence,
and the node reports what is *actually* sounding — a tone ends on its own
when its duration runs out, so the app follows the node's reports rather
than its own taps.

    app -> node   {"tone": {"freq": 440, "ms": 400}}   play one tone
                  {"melody": true}                     play the built-in melody
                  {"stop": true}                       silence
    node -> app   {"freq": 440} / {"freq": 0}          what is sounding (on
                                                       connect and on every
                                                       change)

Over BLE the state arrives as a notify on the data characteristic and the
command as a short binary write on the command characteristic:

    0x01 <freq:u16 big-endian> <ms:u16 big-endian>   play one tone
    0x02                                             play the melody
    0x03                                             silence

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to run without any hardware at all.

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import json
import socket
import sys
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# How often the playback loop advances tones and publishes state changes.
POLL_INTERVAL = 0.02

# Accepted tone range; anything else is ignored as noise.
MIN_FREQ_HZ = 20
MAX_FREQ_HZ = 20000

# The built-in melody: a little C-major fanfare, (freq Hz, duration s).
MELODY = [
    (262, 0.25), (330, 0.25), (392, 0.25), (523, 0.35),
    (392, 0.25), (330, 0.25), (262, 0.50),
]

# -- BLE command framing -----------------------------------------------------

CMD_TONE = 0x01    # 0x01 <freq:u16 BE> <ms:u16 BE>
CMD_MELODY = 0x02  # play the melody
CMD_STOP = 0x03    # silence


# -- Outputs -----------------------------------------------------------------


class PwmOutput:
    """Drives the speaker with a 50% duty square wave via lgpio."""

    def __init__(self, pin, gpio_chip=0):
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
        self._pin = pin
        lgpio.gpio_claim_output(self._handle, pin, 0)

    def play(self, frequency):
        """Sounds [frequency] Hz, or silences the pin when it is 0."""
        if frequency > 0:
            self._lgpio.tx_pwm(self._handle, self._pin, frequency, 50)
        else:
            self._lgpio.tx_pwm(self._handle, self._pin, 1000, 0)

    def close(self):
        self.play(0)
        self._lgpio.gpiochip_close(self._handle)


class EmulatedOutput:
    """Prints the sounding state instead of driving hardware."""

    def play(self, frequency):
        """Sounds [frequency] Hz, or silences the pin when it is 0."""
        state = f"{frequency} Hz" if frequency > 0 else "silent"
        print(f"[emulation] speaker {state}")

    def close(self):
        pass


# -- Playback state ----------------------------------------------------------


class SpeakerController:
    """Owns the sounding state — the single source of truth this node
    publishes.

    Command handlers only stage playback; tick() — driven by the serving
    loop — is what advances melodies and ends tones on time, and every
    change marks the state as pending publication.
    """

    def __init__(self, output, clock):
        self._output = output
        self._clock = clock
        self.frequency = 0
        self._pending = True  # publish the initial state as soon as we serve
        self._deadline = None
        self._melody = []     # remaining (freq, duration) steps
        self._output.play(0)

    def _apply(self, frequency, duration):
        self.frequency = frequency
        self._deadline = (
            self._clock() + duration if frequency > 0 else None
        )
        self._output.play(frequency)
        # Publish unconditionally: a redundant command from a client that
        # guessed wrong would otherwise never be corrected.
        self._pending = True

    def play_tone(self, frequency, milliseconds):
        """Plays one tone, cancelling any melody."""
        if (
            not MIN_FREQ_HZ <= frequency <= MAX_FREQ_HZ
            or milliseconds <= 0
        ):
            print(f"Ignoring tone {frequency} Hz / {milliseconds} ms")
            return
        print(f"Tone: {frequency} Hz for {milliseconds} ms")
        self._melody = []
        self._apply(frequency, milliseconds / 1000.0)

    def play_melody(self):
        """Starts the built-in melody from its first note."""
        print("Melody")
        self._melody = list(MELODY[1:])
        self._apply(*MELODY[0])

    def stop(self):
        """Silences the speaker, whatever it is playing."""
        print("Stop")
        self._melody = []
        self._apply(0, 0)

    def tick(self):
        """Ends a finished tone, or steps through the melody."""
        if self._deadline is None or self._clock() < self._deadline:
            return
        if self._melody:
            self._apply(*self._melody.pop(0))
        else:
            self._apply(0, 0)

    def take_pending(self):
        """Returns True once after each change, clearing the pending flag."""
        pending = self._pending
        self._pending = False
        return pending

    def close(self):
        self._output.close()


# -- Commands ----------------------------------------------------------------


def handle_json_command(speaker, message):
    """Executes one JSON command pushed by the app over the WebSocket."""
    try:
        command = json.loads(message)
    except json.JSONDecodeError:
        print("Ignoring malformed command")
        return

    if command.get("stop") is True:
        speaker.stop()
        return
    if command.get("melody") is True:
        speaker.play_melody()
        return
    tone = command.get("tone")
    if isinstance(tone, dict):
        frequency = tone.get("freq")
        milliseconds = tone.get("ms")
        if isinstance(frequency, int) and isinstance(milliseconds, int):
            speaker.play_tone(frequency, milliseconds)
            return
    print("Ignoring unknown command")


def handle_ble_command(speaker, packet):
    """Executes one binary command packet written over BLE."""
    if not packet:
        raise ValueError("empty command packet")

    opcode = packet[0]
    if opcode == CMD_STOP:
        speaker.stop()
        return
    if opcode == CMD_MELODY:
        speaker.play_melody()
        return
    if opcode != CMD_TONE:
        raise ValueError(f"unknown opcode {opcode:#04x}")
    if len(packet) < 5:
        raise ValueError("tone without frequency and duration")

    frequency = (packet[1] << 8) | packet[2]
    milliseconds = (packet[3] << 8) | packet[4]
    speaker.play_tone(frequency, milliseconds)


# -- Serving -----------------------------------------------------------------

_api_key = ""


async def playback_loop(speaker, publish):
    """Advances playback and publishes every state change.

    Command handlers run inside the transport's dispatch (a D-Bus callback,
    over BLE) and only mutate [speaker]; this loop ends tones on time and
    puts the resulting states on the wire.
    """
    while True:
        speaker.tick()
        if speaker.take_pending():
            await publish({"freq": speaker.frequency})
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(speaker):
    """Serves the WebSocket transport."""

    async def send_current_state(websocket):
        await websocket.send(json.dumps({"freq": speaker.frequency}))

    server = wifi.WsPushServer(
        _api_key,
        on_connect=send_current_state,
        on_message=lambda message: handle_json_command(speaker, message),
    )
    async with server.serve():
        await playback_loop(speaker, server.broadcast)


async def main_ble(speaker, sensor_name, api_key):
    """Serves the BLE transport instead of the WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(
        sensor_name,
        api_key,
        on_command=lambda packet: handle_ble_command(speaker, packet),
    )
    try:
        await transport.start()
        await playback_loop(speaker, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    sensor_name = config.get("sensor_name", "SPEAKER")
    speaker_pin = config.get("speaker_pin", 5)
    gpio_chip = config.get("gpio_chip", 0)

    if config.get("emulation", False):
        print("Emulation mode: printing tones without hardware")
        output = EmulatedOutput()
    else:
        print(f"Initializing speaker on GPIO {speaker_pin}...")
        output = PwmOutput(speaker_pin, gpio_chip=gpio_chip)

    import time

    speaker = SpeakerController(output, time.monotonic)
    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(speaker, sensor_name, _api_key))
            return

        wifi.start_discovery_thread(sensor_name, hostname, wifi.WS_PORT)
        asyncio.run(main_async(speaker))
    finally:
        speaker.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
