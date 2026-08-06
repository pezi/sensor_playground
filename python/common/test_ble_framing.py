"""Tests for the dependency-free BLE notification framing contract."""

import unittest

from ble_framing import (
    FRAME_CONTENT_LENGTH,
    FRAME_END,
    FRAME_HEADER_LENGTH,
    FRAME_MARKER,
    FRAME_START,
    frame_payload,
)


class BleFramingTest(unittest.TestCase):
    def test_payload_round_trips_at_minimum_mtu(self):
        payload = b'{"sensor":"BME280","temperature":21.5,"humidity":48.0}'
        packets = frame_payload(payload, 37)

        self.assertGreater(len(packets), 1)
        self.assertTrue(all(len(packet) <= 20 for packet in packets))
        self.assertEqual(packets[0][0], FRAME_MARKER)
        self.assertEqual(packets[0][1], 37)
        self.assertEqual(packets[0][2], 0)
        self.assertTrue(packets[0][3] & FRAME_START)
        self.assertTrue(packets[-1][3] & FRAME_END)
        self.assertEqual(
            b"".join(packet[FRAME_HEADER_LENGTH:] for packet in packets),
            payload,
        )

    def test_exact_chunk_boundary_marks_final_packet(self):
        payload = bytes(range(FRAME_CONTENT_LENGTH * 2))
        packets = frame_payload(payload, 255)

        self.assertEqual(len(packets), 2)
        self.assertFalse(packets[0][3] & FRAME_END)
        self.assertTrue(packets[1][3] & FRAME_END)

    def test_empty_payload_has_no_notification(self):
        self.assertEqual(frame_payload(b"", 1), [])


if __name__ == "__main__":
    unittest.main()
