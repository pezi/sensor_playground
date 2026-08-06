"""MTU-safe framing for Sensor Tester BLE data notifications.

Each packet is at most 20 bytes, which fits the payload available at BLE's
mandatory 23-byte ATT MTU. The Flutter client also accepts legacy unframed
JSON, so nodes and applications can be upgraded independently.
"""

FRAME_MARKER = 0x1E
FRAME_START = 0x01
FRAME_END = 0x02
FRAME_HEADER_LENGTH = 4
FRAME_CONTENT_LENGTH = 20 - FRAME_HEADER_LENGTH


def frame_payload(payload, message_id):
    """Return ordered notification packets for bytes-like *payload*."""
    payload = bytes(payload)
    if not payload:
        return []

    packets = []
    for chunk_index, offset in enumerate(
        range(0, len(payload), FRAME_CONTENT_LENGTH)
    ):
        if chunk_index > 0xFF:
            raise ValueError("BLE payload needs more than 256 chunks")
        end = min(offset + FRAME_CONTENT_LENGTH, len(payload))
        flags = 0
        if offset == 0:
            flags |= FRAME_START
        if end == len(payload):
            flags |= FRAME_END
        packets.append(
            bytes((FRAME_MARKER, message_id & 0xFF, chunk_index, flags))
            + payload[offset:end]
        )
    return packets
