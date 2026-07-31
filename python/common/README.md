# Shared Transports for Sensor Tester Python Nodes

This folder holds the protocol code shared by every Python sensor node —
each node's `sensor_node.py` supplies only the sensor specifics:

- `wifi_transport.py` — the Wi-Fi side: UDP discovery (port 9133,
  `SENSOR_TESTER` keyword), the HTTPS REST server for pollable nodes
  (Flask) and the WebSocket push server for event/streaming/display nodes
  (both on port 9132, authenticated with the shared `X-Api-Key`).
- `ble_transport.py` — the BLE peripheral (GATT server), used when
  `"transport": "ble"` is set in a node's `config.json`.

Both mirror the corresponding contract of the ESP32 sketches, so the
Sensor Tester app discovers and reads a Python node exactly like an ESP32
node — no app changes required.

The nodes import these modules from the sibling folder (same mechanism as
`extension_hat`), so when deploying a node to a board, copy this `common/`
folder next to the node folder. Flask and websockets are imported lazily by
`wifi_transport.py`, so a node still only needs the packages listed in its
own `requirements.txt`.

The rest of this README documents the BLE transport.

## GATT Contract

| Attribute | UUID | Properties | Purpose |
|-----------|------|------------|---------|
| Service | `d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f` | advertised | Discovery (device name = sensor name, e.g. `MPU6050`) |
| Data characteristic | `d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f` | READ, NOTIFY | UTF-8 JSON payload, identical to the REST/WebSocket body |
| Auth characteristic | `d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f` | WRITE | Client writes the plain API key (`api_key` from `config.json`) |
| Command characteristic | `d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f` | WRITE, WRITE NR | App → node commands, **display nodes only** (see below) |

Flow: connect → write the API key to the auth characteristic → read or
subscribe to the data characteristic. Reads return `{}` until authenticated;
notifications are sent once per second for poll nodes, every 250 ms for the
motion sensors (MMA7660, MPU6050) and per event for push nodes (VL53L0X,
digital contact, PAJ7620) — the same cadences as the ESP32 sketches. On
disconnect the authentication is cleared and the next client must
authenticate again.

A failed sensor read is skipped rather than pushed: clients never receive a
payload that carries only `sensor` and `host`, and a read of the data
characteristic keeps returning the last good payload until the sensor
recovers.

## Display nodes (the command characteristic)

A display node reverses the direction: it consumes data instead of producing
it. `run_ble_commands(name, api_key, on_command)` starts the same peripheral
with a fourth, writable characteristic, and `on_command` is invoked with the
raw bytes of every write once the client has authenticated. Sensor nodes pass
no callback, so the characteristic is not added at all and their GATT layout
is exactly what it has always been.

A display frame does not fit in one write — an ATT write carries at most
MTU-3 bytes — so the app sends a bitmap as a series of chunks and then asks
for it to be drawn. The SSD1306 node defines these as:

| Packet | Meaning |
|--------|---------|
| `0x01 <offset:u16 big-endian> <bytes…>` | Stage a chunk at `offset` |
| `0x02` | Draw the staged frame |
| `0x03` | Blank the display |

Chunks are sent **raw, not base64** — BLE writes are binary-safe, so a
1024-byte frame costs 1024 bytes instead of the ~1.4 KB the Wi-Fi transport's
JSON/base64 form needs. They must arrive contiguously; writing offset 0
starts a new frame, so a client that gives up mid-frame just restarts. A
`0x02` on an incomplete frame is refused rather than drawn, otherwise a
dropped chunk would appear as a band of stale pixels from the previous image.

The transport itself is display-agnostic: the framing above lives in the
node (`FrameAssembler` in `../ssd1306/sensor_node.py`), not here.

Do not raise the poll rate without checking what the sensor does per read: a
BME680 reading fires its gas heater at 320 °C for 150 ms, so polling it at
4 Hz keeps the heater on ~60 % of the time and the reported temperature
climbs several degrees within seconds.

## Prerequisites (Linux / Raspberry Pi)

The transport uses [bless](https://github.com/kevincar/bless), which drives
BlueZ over D-Bus:

- BlueZ >= 5.50 (Raspberry Pi OS Bullseye and later ship a suitable version).
  On older distros the GATT server APIs may require running `bluetoothd`
  with `--experimental`.
- `bluetoothd` running and the adapter powered:

  ```bash
  bluetoothctl power on
  ```

- Registering a GATT application requires D-Bus access to `org.bluez`: run
  the node as `root`, or add the service user to the `bluetooth` group:

  ```bash
  sudo usermod -aG bluetooth pi
  ```

- On Raspberry Pi OS the node currently has to run as **root** (see
  "Advertising fallback" below), so the group membership alone is not enough:

  ```bash
  sudo ./venv/bin/python sensor_node.py
  ```

- If Bluetooth is soft-blocked the adapter cannot be powered at all
  (`bluetoothd` logs `Failed to set mode: Failed (0x03)`); check and clear it:

  ```bash
  rfkill list bluetooth
  sudo rfkill unblock bluetooth
  ```

## Advertising fallback (Raspberry Pi kernels)

Current Raspberry Pi kernels reject the advertising command `bluetoothd`
sends: BlueZ sizes `ADD_EXT_ADV_DATA` with the 11-byte legacy header instead
of the 3-byte extended one, and the kernel refuses the 8 surplus bytes with
`Invalid Parameters (0x0d)`. This affects every BLE peripheral on the board,
including `bluetoothctl advertise on`; `bluetoothd` logs:

```
src/advertising.c:add_client_complete() Failed to add advertisement: Invalid Parameters (0x0d)
```

Only advertising is affected — the GATT server registers fine. When BlueZ
refuses, `ble_transport.py` therefore falls back to `mgmt_advertiser.py`,
which sends the same request to the kernel with the correct length over a
management socket. The node prints:

```
BlueZ advertising failed, advertising via the kernel instead
```

That socket needs `CAP_NET_ADMIN`, which is why the node must run as root.
Once a fixed kernel ships, BlueZ's own path starts working again and the
fallback is simply never taken; `mgmt_advertiser.py` can then be removed.

Notes:

- Only **one** node per board can use the BLE transport, and a second one
  fails in ways that are easy to misread as a flaky app: both processes
  register the GATT application, so the service appears **twice** in the
  database (clients then hit errors such as bleak's "Multiple Characteristics
  with this UUID"), and both claim the same advertising instance, so each
  start tears down the other's advertisement. Check with
  `ps aux | grep sensor_node` before debugging discovery problems — a run left
  in a terminal (even suspended with Ctrl+Z) still counts.
- The node advertises every 100-150 ms. The kernel default of 1.28 s is fine
  for iOS but poor for Android, whose scanner runs in short duty-cycled
  windows and then needs several scans to notice the node.
- The advertised device name sets the Bluetooth adapter alias, i.e. the
  board's Bluetooth name changes to the sensor name while the node runs.
- In BLE mode a node starts neither the HTTPS/WebSocket server nor the UDP
  discovery listener; SSL certificates are not needed.

## Testing

`ble_client_test.py` is a small [bleak](https://github.com/hbldh/bleak)
client that scans for the service, verifies the auth flow and prints
notifications (run it on another machine, or any host with a BLE adapter):

```bash
pip install bleak
python3 ble_client_test.py your-sensor-api-key
```

Expected output: a pre-auth read of `{}`, then JSON notifications (about 4
per second for poll nodes). Alternatively use a generic BLE app such as nRF
Connect: connect, write the API key to `...0003...` as a UTF-8 string, then
subscribe to `...0002...`.

## Troubleshooting

- **No device found while scanning** — check `bluetoothctl show` for
  `Powered: yes`; make sure no other node on the same board is already
  advertising.
- **`BLE advertising could not be started`** — BlueZ refused (expected on
  Raspberry Pi, see above) *and* the kernel fallback failed too. Usually the
  node is not running as root; otherwise check that the adapter is powered
  and not rfkill-blocked.
- **Node advertises but is not discoverable** — another node on the same
  board is advertising, or a previous run was killed and left its instance
  registered. Check with `pgrep -af sensor_node.py` and
  `sudo btmgmt advinfo`; a leftover instance is cleared on the next start.
- **`org.bluez` permission errors on startup** — the user running the node
  is not allowed to register GATT applications; see prerequisites above.
- **Truncated JSON in a generic BLE client** — the payload (~100–150 bytes)
  exceeds the default ATT MTU (23); request a larger MTU in the client (the
  Sensor Tester app does this automatically).
