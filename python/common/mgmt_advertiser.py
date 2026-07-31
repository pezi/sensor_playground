"""
BLE advertising via the kernel's Bluetooth management (mgmt) socket.

Workaround for a Raspberry Pi kernel regression: bluetoothd sends
ADD_EXT_ADV_DATA with 8 trailing bytes (it sizes the buffer with the legacy
``mgmt_cp_add_advertising`` header, 11 bytes, instead of the 3-byte extended
one), and the RPi kernel rejects the over-long command with Invalid Parameters
(0x0d). Every BLE peripheral library is affected, including bluetoothd's own
``bluetoothctl advertise on``, so a sensor node cannot advertise at all.

Registering the GATT server through BlueZ still works, only the advertising
step fails. This module performs that one step directly against the kernel
with a correctly sized command, so the node advertises while BlueZ keeps
serving GATT.

Requires CAP_NET_ADMIN, i.e. the node must run as root. Remove this module
once the RPi kernel ships the fix (see python/common/README.md).
"""

import ctypes
import socket
import struct

# -- Bluetooth management protocol -------------------------------------------

_HCI_CHANNEL_CONTROL = 3
_MGMT_INDEX_NONE = 0xFFFF

_OP_ADD_EXT_ADV_PARAMS = 0x0054
_OP_ADD_EXT_ADV_DATA = 0x0055
_OP_ADD_ADVERTISING = 0x003E
_OP_REMOVE_ADVERTISING = 0x003F

_EV_CMD_COMPLETE = 0x0001
_EV_CMD_STATUS = 0x0002

# mgmt advertising flags
_FLAG_CONNECTABLE = 1 << 0
_FLAG_DISCOVERABLE = 1 << 1
_PARAM_INTERVALS = 1 << 14  # min/max interval in the command are meaningful
_PARAM_SCAN_RSP = 1 << 16  # a scan response follows the advertising data

# Advertising interval in units of 0.625 ms. The kernel's default is 1.28 s,
# which iOS still finds quickly but Android usually misses: its scanner runs
# in short duty-cycled windows, so a slow advertiser needs several scans to be
# discovered. 100-150 ms costs little power and is found on the first scan.
_MIN_INTERVAL = 0x00A0  # 100 ms
_MAX_INTERVAL = 0x00F0  # 150 ms

_AD_UUID128_COMPLETE = 0x07
_AD_NAME_COMPLETE = 0x09

_INSTANCE = 1  # adapter-wide: only one BLE node per board (see README)
_TIMEOUT = 5.0


class MgmtAdvertisingError(Exception):
    """The kernel refused to advertise (or we are not allowed to ask it to)."""


class MgmtAdvertiser:
    """Advertises a service UUID and local name through the mgmt socket."""

    def __init__(self, index=0):
        self.index = index
        self._sock = None
        self._name = None
        self._service_uuid = None
        self._instance = _INSTANCE

    def start(self, name, service_uuid):
        """Advertise ``name`` (scan response) and ``service_uuid`` (adv data)."""
        self._name = name
        self._service_uuid = service_uuid
        self._sock = self._open()
        self._add()

    def restart(self):
        """Re-arm the instance after a client disconnected.

        The controller stops a connectable advertisement once a central
        connects. bluetoothd re-enables its own instances afterwards, but
        nobody re-enables ours, so the node would stay invisible.
        """
        if self._sock is None:
            return
        self._add()

    def _add(self):
        adv_data = _uuid128_ad(self._service_uuid)
        scan_rsp = _name_ad(self._name)
        flags = _FLAG_CONNECTABLE | _FLAG_DISCOVERABLE

        # A SIGKILLed run leaks its instance, so clear ours before claiming it.
        # This is why only one BLE node per board is supported: a second node
        # would take the same instance away from the first.
        self._command(_OP_REMOVE_ADVERTISING, struct.pack("<B", self._instance))

        try:
            self._add_extended(
                flags | _PARAM_INTERVALS | _PARAM_SCAN_RSP, adv_data, scan_rsp
            )
        except MgmtAdvertisingError:
            # Controllers without extended advertising use the legacy command,
            # which has no interval fields: the kernel then advertises at its
            # 1.28 s default and Android clients are slow to discover the node.
            print("BLE: no extended advertising, falling back to a 1.28 s interval")
            self._add_legacy(flags, adv_data, scan_rsp)

    def stop(self):
        """Remove the advertising instance and close the socket."""
        if self._sock is None:
            return
        self._command(_OP_REMOVE_ADVERTISING, struct.pack("<B", self._instance))
        self._sock.close()
        self._sock = None

    # -- mgmt plumbing -------------------------------------------------------

    def _open(self):
        sock = socket.socket(socket.AF_BLUETOOTH, socket.SOCK_RAW, socket.BTPROTO_HCI)
        # Python's bind() cannot express the mgmt control channel, so bind the
        # sockaddr_hci (family, dev, channel) through libc directly.
        libc = ctypes.CDLL("libc.so.6", use_errno=True)
        addr = struct.pack(
            "<HHH", socket.AF_BLUETOOTH, _MGMT_INDEX_NONE, _HCI_CHANNEL_CONTROL
        )
        if libc.bind(sock.fileno(), ctypes.c_char_p(addr), len(addr)) != 0:
            errno = ctypes.get_errno()
            sock.close()
            raise MgmtAdvertisingError(
                f"cannot open the Bluetooth management socket ({errno}); "
                "the node must run as root to advertise"
            )
        sock.settimeout(_TIMEOUT)
        return sock

    def _command(self, opcode, params):
        """Send a mgmt command and return its status byte."""
        self._sock.send(struct.pack("<HHH", opcode, self.index, len(params)) + params)
        while True:
            packet = self._sock.recv(1024)
            event, _, length = struct.unpack("<HHH", packet[:6])
            body = packet[6 : 6 + length]
            if event not in (_EV_CMD_COMPLETE, _EV_CMD_STATUS):
                continue  # unrelated event for another client
            echoed, status = struct.unpack("<HB", body[:3])
            if echoed == opcode:
                return status

    def _add_extended(self, flags, adv_data, scan_rsp):
        params = struct.pack(
            "<BIHHIIb", self._instance, flags, 0, 0, _MIN_INTERVAL, _MAX_INTERVAL, 0
        )
        status = self._command(_OP_ADD_EXT_ADV_PARAMS, params)
        if status != 0:
            raise MgmtAdvertisingError(f"add ext adv params failed (0x{status:02x})")
        data = (
            struct.pack("<BBB", self._instance, len(adv_data), len(scan_rsp))
            + adv_data
            + scan_rsp
        )
        status = self._command(_OP_ADD_EXT_ADV_DATA, data)
        if status != 0:
            raise MgmtAdvertisingError(f"add ext adv data failed (0x{status:02x})")

    def _add_legacy(self, flags, adv_data, scan_rsp):
        data = (
            struct.pack(
                "<BIHHBB", self._instance, flags, 0, 0, len(adv_data), len(scan_rsp)
            )
            + adv_data
            + scan_rsp
        )
        status = self._command(_OP_ADD_ADVERTISING, data)
        if status != 0:
            raise MgmtAdvertisingError(f"add advertising failed (0x{status:02x})")


# -- AD structures -----------------------------------------------------------


def _uuid128_ad(uuid):
    """Complete list of 128-bit service UUIDs (little-endian, as BLE wants)."""
    raw = bytes.fromhex(uuid.replace("-", ""))[::-1]
    return bytes([len(raw) + 1, _AD_UUID128_COMPLETE]) + raw


def _name_ad(name):
    """Complete local name, trimmed to what one AD structure can hold."""
    raw = name.encode("utf-8")[:29]
    return bytes([len(raw) + 1, _AD_NAME_COMPLETE]) + raw
