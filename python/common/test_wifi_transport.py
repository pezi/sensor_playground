"""Tests for WebSocket command responses in the shared Wi-Fi transport."""

import json
import unittest

import wifi_transport as wifi


class _FakeWebSocket:
    def __init__(self, messages):
        self._messages = messages
        self.sent = []

    def __aiter__(self):
        self._iterator = iter(self._messages)
        return self

    async def __anext__(self):
        try:
            return next(self._iterator)
        except StopIteration as exc:
            raise StopAsyncIteration from exc

    async def send(self, message):
        self.sent.append(message)


class WsPushServerTest(unittest.IsolatedAsyncioTestCase):
    async def test_command_result_is_sent_to_the_calling_client(self):
        websocket = _FakeWebSocket(['{"id":5,"clear":true}'])
        server = wifi.WsPushServer(
            "secret",
            on_message=lambda _message: {"id": 5, "ok": True},
        )

        await server._handler(websocket)

        self.assertEqual(json.loads(websocket.sent[0]), {"id": 5, "ok": True})
        self.assertEqual(server._clients, set())

    async def test_async_command_handler_is_awaited(self):
        async def handle(_message):
            return {"id": 8, "ok": False, "error": "display failed"}

        websocket = _FakeWebSocket(["command"])
        server = wifi.WsPushServer("secret", on_message=handle)

        await server._handler(websocket)

        self.assertEqual(
            json.loads(websocket.sent[0]),
            {"id": 8, "ok": False, "error": "display failed"},
        )


if __name__ == "__main__":
    unittest.main()
