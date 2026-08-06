"""
Sensor Tester BLE Transport (Python)

Shared BLE peripheral (GATT server) for the Python sensor nodes, mirroring
the ESP32 sketches' BLE contract so the Sensor Tester app needs no changes:

- Service  d1a51b00-0001-...  advertised under the sensor name
- Data     d1a51b00-0002-...  READ | NOTIFY, JSON payload (same as REST/WS)
- Auth     d1a51b00-0003-...  WRITE, client sends the plain API key
- Command  d1a51b00-0004-...  WRITE, app -> node (display nodes only)

A client connects, writes the API key to the auth characteristic and then
reads or subscribes to the data characteristic. Unauthenticated reads return
"{}" and no notifications are sent; when the client disconnects the
authentication is cleared and a new client must authenticate again.

The command characteristic reverses the direction for nodes that consume
data instead of producing it (the SSD1306 display). It only exists when a
node passes ``on_command``, so sensor nodes keep the three-characteristic
layout they have always advertised.

Sensor nodes import this module from the sibling ``common`` folder (same
mechanism as ``extension_hat``):

    sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))
    import ble_transport
"""

import asyncio
import json
import signal

import mgmt_advertiser
from ble_framing import frame_payload

from bless import (
    BlessServer,
    GATTAttributePermissions,
    GATTCharacteristicProperties,
)

# -- GATT contract (must match the ESP32 sketches and the app) ----------------

SERVICE_UUID = "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
DATA_CHAR_UUID = "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
AUTH_CHAR_UUID = "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
COMMAND_CHAR_UUID = "d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f"

# Seconds between pushes, matching the ESP32 sketches. Sensors are polled at
# 1 Hz: a BME680 reading runs its gas heater at 320 °C for 150 ms, and at 4 Hz
# that heat reaches the temperature die and the reading drifts up by several
# degrees within seconds. Only the motion sensors push faster.
NOTIFY_INTERVAL = 1.0
MOTION_NOTIFY_INTERVAL = 0.25

_DISCONNECT_POLL = 1.0  # seconds between connection checks
_NOTIFICATION_GAP = 0.01  # let the controller transmit each framed packet

# Keys every payload carries; a payload holding only these has no reading.
_METADATA_KEYS = frozenset({"sensor", "host"})


class BleTransport:
    """BLE peripheral publishing sensor JSON per the Sensor Tester contract."""

    def __init__(self, name, api_key, on_command=None):
        self.name = name
        self.api_key = api_key
        self.authed = False
        self._last_payload = b"{}"
        self._server = None
        self._was_connected = False
        self._watch_task = None
        self._advertiser = None
        self._message_id = 0
        # Display nodes pass a callback here; sensor nodes leave it None and
        # the command characteristic is then not added at all.
        self._on_command = on_command

    async def start(self):
        """Register the GATT service and start advertising."""
        self._server = BlessServer(name=self.name)
        self._server.read_request_func = self._on_read
        self._server.write_request_func = self._on_write

        await self._server.add_new_service(SERVICE_UUID)
        # No initial value: CoreBluetooth requires characteristics with a
        # cached value to be read-only, and this one notifies. Reads are
        # served dynamically via _on_read instead.
        await self._server.add_new_characteristic(
            SERVICE_UUID,
            DATA_CHAR_UUID,
            GATTCharacteristicProperties.read | GATTCharacteristicProperties.notify,
            None,
            GATTAttributePermissions.readable,
        )
        await self._server.add_new_characteristic(
            SERVICE_UUID,
            AUTH_CHAR_UUID,
            GATTCharacteristicProperties.write,
            None,
            GATTAttributePermissions.writeable,
        )
        if self._on_command is not None:
            # Accept both write flavours: a frame is many writes, and clients
            # that can use write-without-response send it far faster.
            await self._server.add_new_characteristic(
                SERVICE_UUID,
                COMMAND_CHAR_UUID,
                GATTCharacteristicProperties.write
                | GATTCharacteristicProperties.write_without_response,
                None,
                GATTAttributePermissions.writeable,
            )
        try:
            await self._server.start()
        except Exception as exc:  # BlueZ reports every cause as one DBusError
            # The GATT server is registered by this point; only advertising
            # failed. Raspberry Pi kernels reject the advertising command
            # bluetoothd sends, so drive the kernel directly instead.
            self._advertiser = mgmt_advertiser.MgmtAdvertiser()
            try:
                self._advertiser.start(self.name, SERVICE_UUID)
            except mgmt_advertiser.MgmtAdvertisingError as fallback_exc:
                self._advertiser = None
                raise RuntimeError(
                    f"BLE advertising could not be started: BlueZ said "
                    f"'{exc}' and the kernel fallback failed: {fallback_exc}"
                ) from exc
            print("BlueZ advertising failed, advertising via the kernel instead")
        self._watch_task = asyncio.get_event_loop().create_task(
            self._watch_disconnect()
        )
        print(f"BLE advertising as {self.name} (service {SERVICE_UUID})")

    async def stop(self):
        """Stop advertising and unregister the GATT application."""
        if self._watch_task is not None:
            self._watch_task.cancel()
            self._watch_task = None

        used_fallback = self._advertiser is not None
        if used_fallback:
            self._advertiser.stop()
            self._advertiser = None

        if self._server is None:
            return
        try:
            if used_fallback:
                await self._stop_gatt_only()
            else:
                await self._server.stop()
        except Exception as exc:  # never let shutdown hide behind a traceback
            print(f"BLE server shutdown incomplete: {exc}")
        self._server = None

    async def _stop_gatt_only(self):
        """Tear down the GATT server without touching BlueZ advertising.

        bless records the advertisement before asking BlueZ to register it, so
        after the fallback its stop() would pop that phantom, fail with "Does
        Not Exist" and skip unregistering the application. Do the remaining
        steps of its stop() ourselves instead.
        """
        app = self._server.app
        adapter = self._server.adapter
        await app.set_name(adapter, "")  # restore the adapter's own alias
        await app.unregister(adapter)
        self._server.bus.unexport(app.path, app)

    async def publish(self, payload):
        """Cache a payload dict as JSON and notify authenticated subscribers.

        Payloads carrying no reading are dropped: a failed sensor read would
        otherwise push bare metadata like {"sensor": ..., "host": ...} and
        every client would have to cope with the missing keys. Reads keep
        returning the last good payload until the sensor recovers.
        """
        if not payload or not set(payload) - _METADATA_KEYS:
            return
        self._last_payload = json.dumps(payload).encode("utf-8")
        if not self.authed:
            return
        characteristic = self._server.get_characteristic(DATA_CHAR_UUID)
        self._message_id = (self._message_id + 1) & 0xFF
        for packet in frame_payload(self._last_payload, self._message_id):
            characteristic.value = bytearray(packet)
            self._server.update_value(SERVICE_UUID, DATA_CHAR_UUID)
            await asyncio.sleep(_NOTIFICATION_GAP)

    # -- GATT callbacks (run inside the BlueZ/CoreBluetooth event handling) ---

    def _on_read(self, characteristic):
        # Serve the cached payload so slow sensor reads stay out of the
        # callback; unauthenticated clients only ever see "{}".
        if characteristic.uuid.lower() == DATA_CHAR_UUID and self.authed:
            return bytearray(self._last_payload)
        return bytearray(b"{}")

    def _on_write(self, characteristic, value):
        uuid = characteristic.uuid.lower()
        if uuid == AUTH_CHAR_UUID:
            # Some clients append a trailing NUL to string writes.
            key = bytes(value).decode("utf-8", errors="ignore").rstrip("\x00")
            self.authed = key == self.api_key
            print("BLE client authenticated" if self.authed else "BLE auth rejected")
        elif uuid == COMMAND_CHAR_UUID and self._on_command is not None:
            if not self.authed:
                return  # commands are gated like reads and notifications
            try:
                self._on_command(bytes(value))
            except Exception as exc:
                # This runs inside BlueZ's D-Bus dispatch; letting it raise
                # would surface as an opaque error on the client instead.
                print(f"BLE command failed: {exc}")

    async def _watch_disconnect(self):
        """Clear authentication when the client disconnects (like the ESP32)."""
        while True:
            connected = await self._server.is_connected()
            if not connected and self.authed:
                self.authed = False
                print("BLE client disconnected (auth cleared)")
            if self._was_connected and not connected:
                if self._advertiser is not None:
                    # Connecting stopped the advertisement; put it back on air
                    # so the next client can still find this node.
                    try:
                        self._advertiser.restart()
                    except mgmt_advertiser.MgmtAdvertisingError as exc:
                        print(f"BLE re-advertising failed: {exc}")
            self._was_connected = connected
            await asyncio.sleep(_DISCONNECT_POLL)


async def _serve(transport, tick, interval):
    """Run a started transport until a signal arrives, calling tick() in a loop."""
    await transport.start()
    # Without this a `systemctl stop` would kill us before the cleanup below
    # runs, leaving the advertising instance registered.
    stopping = asyncio.Event()
    loop = asyncio.get_event_loop()
    for sig in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(sig, stopping.set)
    try:
        while not stopping.is_set():
            if tick is not None:
                await tick()
            await asyncio.sleep(interval)
    finally:
        # Release the advertising instance, otherwise it stays registered and
        # keeps advertising a node that is no longer running.
        await transport.stop()
        print("BLE transport stopped")


def run_ble_poll(name, api_key, build_payload, interval=NOTIFY_INTERVAL):
    """Blocking helper for poll nodes: publish build_payload() every interval.

    The payload is only built while an authenticated client is connected,
    matching the ESP32 sketches (no sensor reads without a listener).
    """
    transport = BleTransport(name, api_key)

    async def tick():
        if transport.authed:
            await transport.publish(build_payload())

    asyncio.run(_serve(transport, tick, interval))


def run_ble_commands(name, api_key, on_command):
    """Blocking helper for display nodes: serve the command characteristic.

    The reverse of run_ble_poll — this node produces nothing and only waits
    for the app to write commands, so the loop has no work of its own and
    exists purely to keep the event loop alive until a signal arrives.
    """
    transport = BleTransport(name, api_key, on_command=on_command)
    asyncio.run(_serve(transport, None, _DISCONNECT_POLL))
