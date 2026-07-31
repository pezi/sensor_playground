"""
Sensor Tester Display Node — SSD1306 128x64 OLED (Python)

Implements the *display* variant of the Sensor Tester Sensor Interface on
single-board computers (Raspberry Pi & co.) with an SSD1306 I2C OLED.
Unlike sensor nodes this node consumes data: the app pushes one JSON
command per action over the WebSocket and the node draws it on the panel.

    {"image": "<base64>"}   show a bitmap (1024 bytes, see below)
    {"clear": true}         blank the display

Bitmap format (matches the dart_periphery SSD1306 example):
128x64 pixels, 1 bit per pixel, packed horizontally row by row —
16 bytes per row, MSB of each byte is the leftmost pixel (the
https://javl.github.io/image2cpp/ "horizontal" byte orientation).
The node transposes this into the SSD1306 native page format before
writing it over I2C.

- WebSocket server (ws://) on port 9132, X-Api-Key checked on the handshake
  + UDP Discovery on port 9133 (default), or
- BLE GATT server ("transport": "ble" in config.json)

Over BLE the same two actions arrive as binary writes to the command
characteristic instead of as JSON, because one ATT write carries at most
MTU-3 bytes and a frame is 1024 of them (see FrameAssembler below).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import base64
import binascii
import json
import socket
import sys
from pathlib import Path

from smbus2 import SMBus, i2c_msg

# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Display -----------------------------------------------------------------

WIDTH = 128
HEIGHT = 64
FRAME_SIZE = WIDTH * HEIGHT // 8  # 1024
ROW_BYTES = WIDTH // 8  # 16

# Same init sequence as the dart_periphery SSD1306 driver.
INIT_SEQUENCE = [
    0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00,
    0xA1, 0xC8, 0xDA, 0x12, 0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6,
    0xAF,
]


def to_native_format(data):
    """Transpose a horizontal MSB-first bitmap into SSD1306 page format.

    Port of dart_periphery's SSD1306.displayBitmap: the output has 8 pages
    of 128 column bytes; each byte holds 8 vertical pixels, LSB on top.
    """
    buffer = bytearray(FRAME_SIZE)
    count = 0
    index = 0
    for _page in range(HEIGHT // 8):
        for column_byte in range(ROW_BYTES):
            pos = index + column_byte
            for bit in range(7, -1, -1):
                mask = 1 << bit
                byte = 0
                for row in range(8):
                    if data[pos + ROW_BYTES * row] & mask:
                        byte |= 1 << row
                buffer[count] = byte
                count += 1
        index += WIDTH
    return buffer


class SSD1306Display:
    """Drives an SSD1306 128x64 OLED over I2C (port of dart_periphery)."""

    def __init__(self, i2c_bus=1, i2c_address=0x3C):
        self.bus = SMBus(i2c_bus)
        self.address = i2c_address
        self.name = "SSD1306"
        self._write(0x00, INIT_SEQUENCE)
        self.clear()

    def _write(self, control, data):
        """Write data bytes prefixed with a control byte (0x00/0x40)."""
        message = i2c_msg.write(self.address, [control] + list(data))
        self.bus.i2c_rdwr(message)

    def _reset_position(self):
        """Reset the internal memory write index to the begin."""
        self._write(0x00, [0xB0, 0x00, 0x10])

    def clear(self):
        """Blank the display."""
        self._reset_position()
        self._write(0x40, bytes(FRAME_SIZE))

    def show_bitmap(self, data):
        """Display a horizontal MSB-first bitmap (1024 bytes)."""
        self._reset_position()
        self._write(0x40, to_native_format(data))


# -- BLE command framing -----------------------------------------------------

CMD_CHUNK = 0x01  # 0x01 <offset:u16 big-endian> <bytes...>
CMD_SHOW = 0x02  # draw the staged frame
CMD_CLEAR = 0x03  # blank the display


class FrameAssembler:
    """Reassembles a bitmap from the app's chunked BLE writes.

    BLE has no equivalent of a WebSocket text frame: one ATT write carries at
    most MTU-3 bytes, so a 1024-byte frame arrives as several writes that the
    node has to stage. The chunks are also sent raw rather than base64-encoded
    (BLE writes are binary-safe), which keeps the transfer at 1024 bytes
    instead of the ~1.4 KB the JSON/base64 form would cost.

    Chunks carry an absolute offset and must arrive contiguously. The offset
    is what makes the protocol self-synchronising: writing offset 0 starts a
    new frame, so a client that reconnects or gives up mid-frame simply
    restarts and never has to be told to reset. The contiguity check is what
    makes a partial frame detectable — without it a dropped chunk would be
    drawn as a band of stale pixels from the previous image.
    """

    def __init__(self, frame_size=FRAME_SIZE):
        self.frame_size = frame_size
        self._buffer = bytearray(frame_size)
        self._staged = 0

    def feed(self, packet):
        """Consume one command packet.

        Returns "show" when a complete frame is ready (in .frame), "clear"
        for a clear command, or None when the packet only staged data.
        Raises ValueError on a malformed or out-of-order packet.
        """
        if not packet:
            raise ValueError("empty command packet")

        opcode = packet[0]
        if opcode == CMD_CLEAR:
            self._staged = 0
            return "clear"
        if opcode == CMD_SHOW:
            if self._staged != self.frame_size:
                staged = self._staged
                self._staged = 0
                raise ValueError(
                    f"show with an incomplete frame ({staged} of "
                    f"{self.frame_size} bytes)"
                )
            self._staged = 0
            return "show"
        if opcode != CMD_CHUNK:
            raise ValueError(f"unknown opcode {opcode:#04x}")

        if len(packet) < 3:
            raise ValueError("chunk without an offset")
        offset = (packet[1] << 8) | packet[2]
        data = packet[3:]
        if offset + len(data) > self.frame_size:
            raise ValueError(
                f"chunk at offset {offset} overruns the frame "
                f"({len(data)} bytes)"
            )
        if offset != self._staged:
            # Offset 0 is a deliberate restart, anything else is a gap.
            if offset != 0:
                expected = self._staged
                self._staged = 0
                raise ValueError(
                    f"chunk at offset {offset} is not contiguous "
                    f"(expected {expected})"
                )
        self._buffer[offset:offset + len(data)] = data
        self._staged = offset + len(data)
        return None

    @property
    def frame(self):
        """The staged bitmap, valid right after feed() returned "show"."""
        return bytes(self._buffer)


def make_ble_command_handler(display, assembler=None):
    """Build the on_command callback the BLE transport invokes per write."""
    assembler = assembler or FrameAssembler()

    def on_command(packet):
        action = assembler.feed(packet)  # ValueError is logged by the caller
        if action == "clear":
            print("Command: clear")
            display.clear()
        elif action == "show":
            print("Command: image")
            display.show_bitmap(assembler.frame)

    return on_command


# -- WebSocket command server -------------------------------------------------

_api_key = ""
_display = None


def handle_command(message):
    """Execute one JSON command pushed by the app."""
    try:
        command = json.loads(message)
    except json.JSONDecodeError:
        print("Ignoring malformed command")
        return

    if command.get("clear") is True:
        print("Command: clear")
        _display.clear()
        return

    image = command.get("image")
    if image is None:
        return
    try:
        data = base64.b64decode(image, validate=True)
    except binascii.Error:
        print("Ignoring command with invalid base64 image")
        return
    if len(data) != FRAME_SIZE:
        print(f"Ignoring image with {len(data)} bytes (need {FRAME_SIZE})")
        return
    print("Command: image")
    _display.show_bitmap(data)


async def main_async():
    server = wifi.WsPushServer(_api_key, on_message=handle_command)
    async with server.serve():
        await asyncio.Future()  # serve forever


# -- Main --------------------------------------------------------------------


def main():
    global _api_key, _display

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    i2c_bus = config.get("i2c_bus", 1)
    i2c_address = int(str(config.get("i2c_address", "0x3C")), 16)

    print("Initializing SSD1306 display...")
    _display = SSD1306Display(i2c_bus=i2c_bus, i2c_address=i2c_address)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_commands(
            _display.name, _api_key, make_ble_command_handler(_display)
        )
        return

    wifi.start_discovery_thread(_display.name, hostname, wifi.WS_PORT)

    asyncio.run(main_async())


if __name__ == "__main__":
    main()
