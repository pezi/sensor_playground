"""
Sensor Playground EEPROM Node — AT24C128 (Python)

Implements the *actuator* variant of the Sensor Playground Sensor Interface on
single-board computers (Raspberry Pi & co.) with an AT24C128 serial EEPROM
(128 Kbit / 16 KB, I2C address 0x50). The app stores a short text on the
chip and reads it back at any time; the text survives power cycles of both
ends.

Like the LED the node is the single source of truth: after a write it reads
the chip back and reports the *stored* text, so a failed write cannot leave
the app showing a text the chip never held.

    app -> node   {"write": "Hello"}   store the text on the chip
                  {"read": true}       re-read the chip and push
    node -> app   {"text": "Hello"}    stored text (on connect and after
                                       every write/read, read from the chip)

EEPROM layout (offset 0): magic 'S' 'P', u16 big-endian text length
(max 512 bytes), then the UTF-8 text. A chip without the magic (e.g.
factory-fresh, all 0xFF) reads as an empty text.

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Over BLE the stored text arrives as a notify on the data characteristic
carrying the same JSON, and a write is staged in offset-addressed binary
chunks on the command characteristic (like the SSD1306 bitmap), because a
text may not fit in a single ATT write:

    0x01 <offset:u16 big-endian> <bytes...>   stage a chunk of UTF-8 text
    0x02 <length:u16 big-endian>              store the staged text
    0x03                                      re-read the chip and push

Set "emulation": true in config.json to run without the chip (the text then
lives in memory only).

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

# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# Seconds between pending-publication polls (commands only mark the state
# dirty; this loop is what puts it on the wire).
POLL_INTERVAL = 0.05

# EEPROM text region: 2-byte magic + 2-byte length + up to 512 bytes UTF-8.
MAGIC = b"SP"
HEADER_SIZE = 4
TEXT_MAX_BYTES = 512

# AT24C128 write page; a write transaction must not cross a page boundary.
PAGE_SIZE = 64

# Bytes per I2C transaction, safe on every adapter.
IO_CHUNK = 32

# Worst-case internal write cycle per the datasheet is 5 ms.
WRITE_CYCLE_S = 0.006

# -- BLE command framing -----------------------------------------------------

CMD_CHUNK = 0x01  # 0x01 <offset:u16 big-endian> <bytes...>
CMD_WRITE = 0x02  # 0x02 <length:u16 big-endian>
CMD_READ = 0x03   # re-read the chip and push


# -- EEPROM hardware ---------------------------------------------------------


class At24c128Eeprom:
    """Reads and writes the AT24C128 text region over I2C (16-bit
    addressing)."""

    def __init__(self, i2c_bus=1, i2c_address=0x50):
        from smbus2 import SMBus

        self.bus = SMBus(i2c_bus)
        self.address = i2c_address
        self.read_text()  # fail fast on a wiring/address problem

    def _read(self, addr, length):
        """Random-reads [length] bytes starting at [addr]."""
        from smbus2 import i2c_msg

        data = bytearray()
        for offset in range(0, length, IO_CHUNK):
            at = addr + offset
            count = min(IO_CHUNK, length - offset)
            write = i2c_msg.write(self.address, [at >> 8, at & 0xFF])
            read = i2c_msg.read(self.address, count)
            self.bus.i2c_rdwr(write, read)
            data.extend(bytes(read))
        return bytes(data)

    def _write(self, addr, data):
        """Writes [data] starting at [addr], splitting the transactions so
        none crosses a 64-byte page boundary, and waiting out the chip's
        internal write cycle after each one."""
        from smbus2 import i2c_msg

        offset = 0
        while offset < len(data):
            at = addr + offset
            page_left = PAGE_SIZE - (at % PAGE_SIZE)
            count = min(IO_CHUNK, page_left, len(data) - offset)
            chunk = data[offset : offset + count]
            message = i2c_msg.write(
                self.address, [at >> 8, at & 0xFF] + list(chunk)
            )
            self.bus.i2c_rdwr(message)
            time.sleep(WRITE_CYCLE_S)
            offset += count

    def read_text(self):
        """Reads the stored text from the chip.

        A missing magic or an implausible length reads as an empty text
        rather than as garbage.
        """
        header = self._read(0, HEADER_SIZE)
        if header[:2] != MAGIC:
            return ""
        length = (header[2] << 8) | header[3]
        if length > TEXT_MAX_BYTES:
            return ""
        return self._read(HEADER_SIZE, length).decode(
            "utf-8", errors="replace"
        )

    def write_text(self, text_bytes):
        """Stores the UTF-8 [text_bytes] (header + payload) on the chip."""
        header = MAGIC + bytes(
            [(len(text_bytes) >> 8) & 0xFF, len(text_bytes) & 0xFF]
        )
        self._write(0, header + text_bytes)

    def close(self):
        self.bus.close()


class EmulatedEeprom:
    """Keeps the text in memory instead of driving hardware."""

    def __init__(self):
        self._text = "Hello from the emulated EEPROM"

    def read_text(self):
        """Returns the fake stored text."""
        return self._text

    def write_text(self, text_bytes):
        """Stores the UTF-8 [text_bytes] in memory."""
        self._text = text_bytes.decode("utf-8", errors="replace")
        print(f"[emulation] stored {len(text_bytes)} bytes")

    def close(self):
        pass


# -- Stored-text state -------------------------------------------------------


class EepromController:
    """Owns the stored text — the single source of truth this node
    publishes.

    Every command marks the state as pending publication, even one that does
    not change it: the published text is always a fresh read-back, so the
    app renders what the chip actually holds.
    """

    def __init__(self, eeprom):
        self._eeprom = eeprom
        self.text = eeprom.read_text()
        self._pending = True  # publish the initial text as soon as we serve

    def write(self, text_bytes):
        """Stores [text_bytes] and re-reads the chip."""
        if len(text_bytes) > TEXT_MAX_BYTES:
            print(
                f"Rejecting write of {len(text_bytes)} bytes "
                f"(max {TEXT_MAX_BYTES})"
            )
        else:
            self._eeprom.write_text(text_bytes)
        self.read()

    def read(self):
        """Re-reads the chip and marks the text for publication."""
        self.text = self._eeprom.read_text()
        self._pending = True

    def take_pending(self):
        """Returns True once after each command, clearing the pending
        flag."""
        pending = self._pending
        self._pending = False
        return pending

    def close(self):
        self._eeprom.close()


# -- Commands ----------------------------------------------------------------


def handle_json_command(controller, message):
    """Executes one JSON command pushed by the app over the WebSocket."""
    try:
        command = json.loads(message)
    except json.JSONDecodeError:
        print("Ignoring malformed command")
        return

    if command.get("read") is True:
        print("Command: read")
        controller.read()
        return

    text = command.get("write")
    if not isinstance(text, str):
        print("Ignoring command without a string 'write'")
        return
    print(f"Command: write {len(text)} characters")
    controller.write(text.encode("utf-8"))


class BleCommandStream:
    """Feeds command-characteristic writes into the controller.

    Chunks carry an absolute offset and must arrive contiguously. The offset
    is what makes the protocol self-synchronising: staging offset 0 starts a
    new transfer, and the store command carries the total length, so a
    dropped packet refuses the write instead of storing a torn text.
    """

    def __init__(self, controller):
        self._controller = controller
        self._buffer = bytearray(TEXT_MAX_BYTES)
        self._staged = 0

    def feed(self, packet):
        """Handles one command packet, raising ValueError on a bad one."""
        if not packet:
            raise ValueError("empty command packet")

        opcode = packet[0]
        if opcode == CMD_READ:
            print("Command: read")
            self._controller.read()
            return
        if opcode == CMD_WRITE:
            self._store(packet)
            return
        if opcode != CMD_CHUNK:
            raise ValueError(f"unknown opcode {opcode:#04x}")
        self._stage(packet)

    def _store(self, packet):
        if len(packet) < 3:
            raise ValueError("write without a length")
        total = (packet[1] << 8) | packet[2]
        staged = self._staged
        self._staged = 0
        if total != staged:
            raise ValueError(
                f"write of {total} bytes with {staged} staged"
            )
        print(f"Command: write {total} bytes")
        self._controller.write(bytes(self._buffer[:total]))

    def _stage(self, packet):
        if len(packet) < 3:
            raise ValueError("chunk without an offset")
        offset = (packet[1] << 8) | packet[2]
        data = packet[3:]
        if offset + len(data) > TEXT_MAX_BYTES:
            raise ValueError(
                f"chunk at offset {offset} overruns the text region "
                f"({len(data)} bytes)"
            )
        if offset != self._staged:
            if offset != 0:
                expected = self._staged
                self._staged = 0
                raise ValueError(
                    f"chunk at offset {offset} is not contiguous "
                    f"(expected {expected})"
                )
            self._staged = 0
        self._buffer[offset : offset + len(data)] = data
        self._staged = offset + len(data)


# -- Serving -----------------------------------------------------------------

_api_key = ""


async def text_loop(controller, publish):
    """Publishes the stored text whenever a command marked it pending.

    Command handlers run inside the transport's dispatch (a D-Bus callback,
    over BLE), where publishing directly would mean reaching across into the
    event loop — so they only mutate [controller] and this loop puts the
    result on the wire.
    """
    while True:
        if controller.take_pending():
            print(f"text: {controller.text!r}")
            await publish({"text": controller.text})
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(controller):
    """Serves the WebSocket transport."""

    async def send_current_text(websocket):
        # The chip holds state: a client that just connected gets the
        # stored text instead of an empty field.
        await websocket.send(json.dumps({"text": controller.text}))

    server = wifi.WsPushServer(
        _api_key,
        on_connect=send_current_text,
        on_message=lambda message: handle_json_command(controller, message),
    )
    async with server.serve():
        await text_loop(controller, server.broadcast)


async def main_ble(controller, sensor_name, api_key):
    """Serves the BLE transport instead of the WebSocket (see ../common)."""
    import ble_transport

    commands = BleCommandStream(controller)
    transport = ble_transport.BleTransport(
        sensor_name,
        api_key,
        on_command=commands.feed,
    )
    try:
        await transport.start()
        await text_loop(controller, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    sensor_name = config.get("sensor_name", "AT24C128")
    i2c_bus = config.get("i2c_bus", 1)
    i2c_address = int(str(config.get("i2c_address", "0x50")), 16)

    if config.get("emulation", False):
        print("Emulation mode: storing the text in memory without hardware")
        eeprom = EmulatedEeprom()
    else:
        print(f"Opening AT24C128 on i2c bus {i2c_bus}...")
        eeprom = At24c128Eeprom(i2c_bus=i2c_bus, i2c_address=i2c_address)

    controller = EepromController(eeprom)
    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(controller, sensor_name, _api_key))
            return

        wifi.start_discovery_thread(sensor_name, hostname, wifi.WS_PORT)
        asyncio.run(main_async(controller))
    finally:
        controller.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
