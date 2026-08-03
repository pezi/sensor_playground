"""
Sensor Tester BLE Client Check (Python)

Small bleak client to verify a BLE sensor node (Python or ESP32): scans for
the Sensor Tester service, connects, exercises the auth flow and prints
notifications for a few seconds.

Usage:
    pip install bleak
    python3 ble_client_test.py <api-key> [device-name]
"""

import asyncio
import sys

from bleak import BleakClient, BleakScanner

SERVICE_UUID = "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
DATA_CHAR_UUID = "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
AUTH_CHAR_UUID = "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"

LISTEN_SECONDS = 5.0


async def main():
    if len(sys.argv) < 2:
        sys.exit("Usage: python3 ble_client_test.py <api-key> [device-name]")
    api_key = sys.argv[1]
    name = sys.argv[2] if len(sys.argv) > 2 else None

    print("Scanning for Sensor Tester service...")
    device = await BleakScanner.find_device_by_filter(
        lambda d, ad: SERVICE_UUID in (ad.service_uuids or [])
        and (name is None or d.name == name),
        timeout=15.0,
    )
    if device is None:
        sys.exit("No Sensor Tester BLE device found")
    print(f"Connecting to {device.name} ({device.address})")

    async with BleakClient(device) as client:
        data = await client.read_gatt_char(DATA_CHAR_UUID)
        print(f"Pre-auth read (expect {{}}): {data.decode()}")

        await client.write_gatt_char(AUTH_CHAR_UUID, api_key.encode(), response=True)

        count = 0

        def on_notify(_, payload):
            nonlocal count
            count += 1
            print(f"notify: {payload.decode()}")

        await client.start_notify(DATA_CHAR_UUID, on_notify)
        await asyncio.sleep(LISTEN_SECONDS)
        await client.stop_notify(DATA_CHAR_UUID)

        data = await client.read_gatt_char(DATA_CHAR_UUID)
        print(f"Post-auth read: {data.decode()}")
        print(f"{count} notifications in {LISTEN_SECONDS:.0f} s")


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("Stopped.")
