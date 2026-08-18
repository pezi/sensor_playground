"""
Sensor Tester Sensor Node — Grove 125KHz RFID Reader (Python)

Implements the *push* variant of the Sensor Tester Sensor Interface on
single-board computers (Raspberry Pi & co.) with a Grove 125KHz RFID
Reader. The node reads RDM630-style frames from a 9600-baud UART and
pushes one JSON message ({"tag": "0F0024ADAB"}) per scanned EM4100 tag.

Frame format (reader TX, jumper on UART mode — not Wiegand):
    STX 0x02 | 10 ASCII-hex data chars | 2 ASCII-hex checksum chars | ETX 0x03
The checksum byte is the XOR of the five data bytes. The reader repeats
the frame while a tag is held near the antenna, so the node suppresses
repeats of the same tag for REPEAT_SUPPRESS_S seconds.

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible scans without
the reader hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import random
import socket
import sys
import time
from pathlib import Path

# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

POLL_INTERVAL = 0.05

# Suppress repeats of the same tag while it is held near the antenna.
REPEAT_SUPPRESS_S = 2.0

STX = 0x02
ETX = 0x03
FRAME_HEX_CHARS = 12  # 10 data chars + 2 checksum chars

# -- Sensor ------------------------------------------------------------------


class RdmRfidReader:
    """Reads RDM630-style frames from a Grove 125KHz RFID Reader UART."""

    def __init__(self, serial_port="/dev/serial0"):
        import serial

        # Non-blocking: read_tag() drains whatever arrived since last poll.
        self.ser = serial.Serial(serial_port, 9600, timeout=0)
        self.name = "RFID"
        self._frame = None  # None = waiting for STX, else collected hex chars

    def read_tag(self):
        """Drain the port; return a validated 10-char hex tag or None."""
        result = None
        data = self.ser.read(64)
        for byte in data:
            tag = self._feed(byte)
            if tag is not None:
                result = tag
        return result

    def _feed(self, byte):
        """Advance the frame state machine by one byte."""
        if byte == STX:
            self._frame = bytearray()  # resync, also on a second STX
            return None
        if self._frame is None:
            return None  # noise outside a frame
        if byte == ETX:
            frame = self._frame
            self._frame = None
            if len(frame) == FRAME_HEX_CHARS and _checksum_ok(frame):
                return frame[:10].decode("ascii").upper()
            return None
        if chr(byte).upper() in "0123456789ABCDEF":
            self._frame.append(byte)
            if len(self._frame) > FRAME_HEX_CHARS:
                self._frame = None  # overflow: wait for the next STX
        else:
            self._frame = None  # non-hex noise mid-frame
        return None


def _checksum_ok(frame):
    """XOR of the five data bytes must equal the checksum byte."""
    values = [int(frame[i : i + 2], 16) for i in range(0, FRAME_HEX_CHARS, 2)]
    checksum = 0
    for value in values[:5]:
        checksum ^= value
    return checksum == values[5]


class EmulatedRfidReader(RdmRfidReader):
    """Generates plausible RFID scans without hardware.

    Reports one tag from a small fixed pool every four to eight seconds;
    polls in between return no tag.
    """

    _TAGS = ["0F0024ADAB", "0A0031B2C4", "03004F19AA", "1000C0FFEE"]

    def __init__(self):
        self.name = "RFID"
        self._next_at = time.time() + random.uniform(4.0, 8.0)

    def read_tag(self):
        """Return a scanned tag, or None if nothing happened."""
        if time.time() < self._next_at:
            return None
        self._next_at = time.time() + random.uniform(4.0, 8.0)
        return random.choice(self._TAGS)


# -- WebSocket push server ----------------------------------------------------

_api_key = ""


async def tag_loop(reader, publish):
    """Poll the reader and push each scanned tag, suppressing repeats."""
    last_tag = None
    last_at = 0.0
    while True:
        tag = reader.read_tag()
        if tag is not None and (
            tag != last_tag or time.time() - last_at >= REPEAT_SUPPRESS_S
        ):
            print(f"Tag: {tag}")
            await publish({"tag": tag})
            last_tag = tag
            last_at = time.time()
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(reader):
    server = wifi.WsPushServer(_api_key)
    async with server.serve():
        await tag_loop(reader, server.broadcast)


async def main_ble(reader, api_key):
    """Push scans over BLE instead of WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(reader.name, api_key)
    await transport.start()
    try:
        await tag_loop(reader, transport.publish)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    serial_port = config.get("serial_port", "/dev/serial0")

    if config.get("emulation", False):
        print("Emulation mode: generating RFID scans without hardware")
        reader = EmulatedRfidReader()
    else:
        print(f"Opening RFID reader on {serial_port}...")
        reader = RdmRfidReader(serial_port=serial_port)

    if config.get("transport", "wifi") == "ble":
        asyncio.run(main_ble(reader, _api_key))
        return

    wifi.start_discovery_thread(reader.name, hostname, wifi.WS_PORT)

    asyncio.run(main_async(reader))


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
