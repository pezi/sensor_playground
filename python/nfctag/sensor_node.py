"""
Sensor Playground Sensor Node — Grove NFC Tag (Python)

Implements the *push* variant of the Sensor Playground Sensor Interface on
single-board computers (Raspberry Pi & co.) with a Grove NFC Tag — a
passive dual-interface EEPROM (ST M24LR64E-R, 8 KB). A phone or NFC
writer stores an NDEF message over the ISO 15693 RF interface; this node
reads the same memory over I2C, parses the first NDEF record and pushes
one JSON message whenever the content changes:

    {"kind": "text", "value": "Hello"}
    {"kind": "uri",  "value": "https://seeed.cc"}
    {"kind": "data", "value": "DEADBEEF"}      (hex, truncated)
    {"kind": "empty"}

Unlike the pure event sensors the tag holds state, so the current
content is also sent to every client right after it connects (and served
on BLE reads).

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to cycle through generated contents
without the tag hardware (works with both transports).

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

# Seconds between EEPROM polls; an RF write shows up on the next poll.
POLL_INTERVAL = 1.0

# Bytes of the EEPROM scanned for the NDEF message (CC + TLV area).
SCAN_LENGTH = 256

# Cap for hex dumps of unparseable payloads (bytes before hex encoding).
DATA_HEX_CAP = 64

# NFC Forum URI record prefix codes (the common subset; the same table
# lives in the ESP32 sketch — keep them identical).
URI_PREFIXES = {
    0x00: "",
    0x01: "http://www.",
    0x02: "https://www.",
    0x03: "http://",
    0x04: "https://",
    0x05: "tel:",
    0x06: "mailto:",
}

# -- NDEF parsing ------------------------------------------------------------


def parse_ndef_area(data):
    """Parses the scanned EEPROM area into a {"kind", "value"} payload.

    The area starts with the Type 5 capability container (magic 0xE1/0xE2),
    followed by a TLV stream in which 0x03 marks the NDEF message. Anything
    that fails to parse is reported as a hex dump rather than dropped, so
    the app always sees that *something* was written.
    """
    if len(data) < 4 or data[0] not in (0xE1, 0xE2):
        if all(byte in (0x00, 0xFF) for byte in data):
            return {"kind": "empty"}
        return _data_payload(data)

    offset = 4  # first byte after the 4-byte capability container
    while offset < len(data):
        tlv = data[offset]
        if tlv == 0x00:  # padding
            offset += 1
            continue
        if tlv == 0xFE:  # terminator: no NDEF TLV found
            return {"kind": "empty"}
        if tlv != 0x03:  # unknown TLV: skip it (1-byte length format)
            if offset + 1 >= len(data):
                return {"kind": "empty"}
            offset += 2 + data[offset + 1]
            continue
        # NDEF message TLV: 1-byte length, or 0xFF + 2-byte big-endian.
        if offset + 1 >= len(data):
            return {"kind": "empty"}
        length = data[offset + 1]
        offset += 2
        if length == 0xFF:
            if offset + 2 > len(data):
                return {"kind": "empty"}
            length = (data[offset] << 8) | data[offset + 1]
            offset += 2
        if length == 0:
            return {"kind": "empty"}
        message = data[offset : offset + length]
        if len(message) < length:
            return _data_payload(message)  # truncated by the scan window
        return _parse_ndef_record(message)
    return {"kind": "empty"}


def _parse_ndef_record(message):
    """Parses the first record of an NDEF message."""
    try:
        flags = message[0]
        tnf = flags & 0x07
        short_record = bool(flags & 0x10)
        has_id = bool(flags & 0x08)
        type_length = message[1]
        offset = 2
        if short_record:
            payload_length = message[offset]
            offset += 1
        else:
            payload_length = int.from_bytes(message[offset : offset + 4], "big")
            offset += 4
        id_length = 0
        if has_id:
            id_length = message[offset]
            offset += 1
        record_type = bytes(message[offset : offset + type_length])
        offset += type_length + id_length
        payload = bytes(message[offset : offset + payload_length])
        if len(payload) < payload_length:
            return _data_payload(payload)

        if tnf == 0x01 and record_type == b"T":
            status = payload[0]
            lang_length = status & 0x3F
            encoding = "utf-16" if status & 0x80 else "utf-8"
            text = payload[1 + lang_length :].decode(encoding, errors="replace")
            return {"kind": "text", "value": text}
        if tnf == 0x01 and record_type == b"U":
            prefix = URI_PREFIXES.get(payload[0], "")
            rest = payload[1:].decode("utf-8", errors="replace")
            return {"kind": "uri", "value": prefix + rest}
        return _data_payload(payload)
    except (IndexError, ValueError):
        return _data_payload(bytes(message))


def _data_payload(data):
    """Hex-dump fallback for content that is not a text or URI record."""
    return {"kind": "data", "value": bytes(data[:DATA_HEX_CAP]).hex().upper()}


# -- Tag hardware ------------------------------------------------------------


class M24lr64Tag:
    """Reads the M24LR64E-R user memory over I2C (16-bit addressing)."""

    _CHUNK = 32  # bytes per I2C transaction, safe on every adapter

    def __init__(self, i2c_bus=1, i2c_address=0x53):
        from smbus2 import SMBus

        self.bus = SMBus(i2c_bus)
        self.address = i2c_address
        self.name = "NFCTAG"
        self.read_content()  # fail fast on a wiring/address problem

    def read_content(self):
        """Scans the NDEF area and returns the parsed payload dict."""
        from smbus2 import i2c_msg

        data = bytearray()
        for offset in range(0, SCAN_LENGTH, self._CHUNK):
            write = i2c_msg.write(self.address, [offset >> 8, offset & 0xFF])
            read = i2c_msg.read(self.address, self._CHUNK)
            self.bus.i2c_rdwr(write, read)
            data.extend(bytes(read))
        return parse_ndef_area(bytes(data))

    def close(self):
        self.bus.close()


class EmulatedNfcTag(M24lr64Tag):
    """Cycles through generated tag contents without hardware."""

    _CONTENTS = [
        {"kind": "text", "value": "Hello from Sensor Playground"},
        {"kind": "uri", "value": "https://wiki.seeedstudio.com/Grove_NFC_Tag/"},
        {"kind": "empty"},
    ]

    def __init__(self):
        self.name = "NFCTAG"
        self._index = 0
        self._next_at = time.time() + 15.0

    def read_content(self):
        """Returns the current fake content, advancing every 15 seconds."""
        if time.time() >= self._next_at:
            self._index = (self._index + 1) % len(self._CONTENTS)
            self._next_at = time.time() + 15.0
        return self._CONTENTS[self._index]

    def close(self):
        pass


# -- WebSocket push server ----------------------------------------------------

_api_key = ""


async def content_loop(tag, publish, state):
    """Poll the tag and push the content whenever it changes."""
    while True:
        try:
            content = tag.read_content()
        except OSError as exc:
            print(f"Tag read failed: {exc}")
            content = None
        if content is not None and content != state.get("content"):
            print(f"Content: {json.dumps(content)}")
            state["content"] = content
            await publish(content)
        await asyncio.sleep(POLL_INTERVAL)


async def main_async(tag, state):
    async def send_current(websocket):
        # The tag holds state: a client that just connected gets the
        # current content instead of waiting for the next RF write.
        content = state.get("content")
        if content is not None:
            await websocket.send(json.dumps(content))

    server = wifi.WsPushServer(_api_key, on_connect=send_current)
    async with server.serve():
        await content_loop(tag, server.broadcast, state)


async def main_ble(tag, api_key, state):
    """Push content over BLE instead of WebSocket (see ../common)."""
    import ble_transport

    transport = ble_transport.BleTransport(tag.name, api_key)
    await transport.start()
    try:
        # Seed the cached payload so a read right after connecting serves
        # the current content; the loop then publishes every change.
        await content_loop(tag, transport.publish, state)
    finally:
        await transport.stop()


# -- Main --------------------------------------------------------------------


def main():
    global _api_key

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    hostname = config.get("hostname", "") or socket.gethostname()
    i2c_bus = config.get("i2c_bus", 1)
    i2c_address = int(str(config.get("i2c_address", "0x53")), 16)

    if config.get("emulation", False):
        print("Emulation mode: cycling NFC tag contents without hardware")
        tag = EmulatedNfcTag()
    else:
        print(f"Opening M24LR64E on i2c bus {i2c_bus}...")
        tag = M24lr64Tag(i2c_bus=i2c_bus, i2c_address=i2c_address)

    state = {}
    try:
        if config.get("transport", "wifi") == "ble":
            asyncio.run(main_ble(tag, _api_key, state))
            return

        wifi.start_discovery_thread(tag.name, hostname, wifi.WS_PORT)

        asyncio.run(main_async(tag, state))
    finally:
        tag.close()


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
