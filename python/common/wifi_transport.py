"""
Shared Wi-Fi transport for Sensor Tester python nodes.

Every node speaks the same Wi-Fi protocol:

- UDP discovery on port 9133: reply to a "SENSOR_TESTER" broadcast with the
  node identity (plus short-key readings for pollable nodes), and
- either an HTTPS REST endpoint (pollable nodes, Flask) or a WebSocket push
  server (event/streaming/display nodes) on port 9132, both authenticated
  with the shared X-Api-Key.

The per-node sensor_node.py supplies only the sensor specifics; this module
holds the protocol so a change lands in one place instead of once per node.
Flask and websockets are imported lazily, so a node only needs the package
its own transport uses (see each node's requirements.txt).

Usage (the sibling ble_transport.py is the BLE counterpart):

    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))
    import wifi_transport as wifi

    config = wifi.load_config(Path(__file__).parent)

    # Pollable node (HTTPS REST):
    wifi.start_discovery_thread(name, host, wifi.HTTPS_PORT, s.read_discovery)
    wifi.run_rest_server(name, api_key, host, s.read, ssl_cert, ssl_key)

    # Push/display node (WebSocket):
    wifi.start_discovery_thread(name, host, wifi.WS_PORT)
    server = wifi.WsPushServer(api_key)
    async with server.serve():
        await event_loop(server.broadcast)
"""

import hmac
import json
import socket
import sys
import threading
from pathlib import Path

# -- Ports (from sensor spec) -----------------------------------------------

HTTPS_PORT = 9132
WS_PORT = 9132
UDP_PORT = 9133
SCAN_KEYWORD = "SENSOR_TESTER"


def load_config(node_dir):
    """Load config.json from the node directory [node_dir]."""
    config_path = Path(node_dir) / "config.json"
    if not config_path.exists():
        print(
            "Error: config.json not found.\n"
            "Copy config.example.json to config.json and edit it."
        )
        sys.exit(1)
    with open(config_path) as f:
        return json.load(f)


def get_local_ip():
    """Determine the local network IP address."""
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    try:
        s.connect(("8.8.8.8", 80))
        return s.getsockname()[0]
    finally:
        s.close()


def _key_matches(supplied, api_key):
    """Constant-time comparison of a supplied API key against the real one."""
    return hmac.compare_digest(supplied or "", api_key)


def udp_discovery_listener(sensor_name, hostname, port, read_discovery=None):
    """Listen for SENSOR_TESTER broadcasts and reply with the node identity.

    Pollable nodes pass [read_discovery] to advertise short-key readings in
    the reply; push and display nodes advertise their identity only.
    """
    sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    sock.bind(("", UDP_PORT))
    print(f"UDP discovery listening on port {UDP_PORT}")

    while True:
        data, addr = sock.recvfrom(1024)
        msg = data.decode("utf-8", errors="ignore").strip()
        if msg != SCAN_KEYWORD:
            continue

        response = {
            "type": sensor_name,
            "host": hostname,
            "ip": get_local_ip(),
            "port": port,
        }
        # One failed reply (e.g. an I2C hiccup inside read_discovery) must
        # not kill the listener thread and leave the node undiscoverable.
        try:
            if read_discovery is not None:
                response.update(read_discovery())
            sock.sendto(json.dumps(response).encode(), addr)
        except Exception as e:
            print(f"Discovery reply failed: {e}")


def start_discovery_thread(sensor_name, hostname, port, read_discovery=None):
    """Run udp_discovery_listener in a daemon thread and return the thread."""
    thread = threading.Thread(
        target=udp_discovery_listener,
        args=(sensor_name, hostname, port, read_discovery),
        daemon=True,
    )
    thread.start()
    return thread


def run_rest_server(
    sensor_name, api_key, hostname, read, ssl_cert, ssl_key, allow_empty=False
):
    """Serve read() over HTTPS REST with X-Api-Key auth. Blocks forever.

    [allow_empty] returns 200 with a metadata-only body when read() yields
    nothing, instead of 503. Used by the GPS node, whose fixless warm-up
    state is normal rather than an error.
    """
    from flask import Flask, jsonify, request

    app = Flask(sensor_name)

    @app.route("/", methods=["GET"])
    def get_sensor_data():
        if not _key_matches(request.headers.get("X-Api-Key"), api_key):
            return "", 401

        data = read()
        if data is None and not allow_empty:
            return jsonify({"error": "Sensor read failed"}), 503

        response = {"sensor": sensor_name, "host": hostname}
        response.update(data or {})
        return jsonify(response)

    print(f"HTTPS server on port {HTTPS_PORT}")
    app.run(host="0.0.0.0", port=HTTPS_PORT, ssl_context=(ssl_cert, ssl_key))


class WsPushServer:
    """WebSocket server with X-Api-Key handshake auth.

    - broadcast(payload) pushes a JSON payload to every connected client
      (event and streaming nodes),
    - [on_connect] (async, optional) runs once per new client, e.g. to send
      the current state (contact node),
    - [on_message] (optional) handles each incoming message instead of just
      waiting for the disconnect (display node).
    """

    def __init__(self, api_key, on_connect=None, on_message=None):
        self._api_key = api_key
        self._on_connect = on_connect
        self._on_message = on_message
        self._clients = set()

    def _process_request(self, connection, request):
        """Reject WebSocket handshakes without the correct X-Api-Key."""
        import http

        if not _key_matches(request.headers.get("X-Api-Key"), self._api_key):
            return connection.respond(http.HTTPStatus.UNAUTHORIZED, "")
        return None

    async def _handler(self, websocket):
        self._clients.add(websocket)
        print("Client connected")
        try:
            if self._on_connect is not None:
                await self._on_connect(websocket)
            if self._on_message is not None:
                async for message in websocket:
                    self._on_message(message)
            else:
                await websocket.wait_closed()
        finally:
            self._clients.discard(websocket)
            print("Client disconnected")

    async def broadcast(self, payload):
        """Send a JSON payload to all connected clients."""
        import websockets

        message = json.dumps(payload)
        for ws in set(self._clients):
            try:
                await ws.send(message)
            except websockets.ConnectionClosed:
                self._clients.discard(ws)

    def serve(self):
        """Async context manager serving on WS_PORT (websockets.serve)."""
        import websockets

        print(f"WebSocket server on port {WS_PORT}")
        return websockets.serve(
            self._handler,
            "0.0.0.0",
            WS_PORT,
            process_request=self._process_request,
        )
