# CameraWebServerBLE — ESP32-CAM BLE Node

> **Experimental.** BLE support for the ESP32-CAM is still experimental.

BLE-only variant of the [`CameraWebServer`](../CameraWebServer/) (plain
HTTP) and [`CameraWebServerWS`](../CameraWebServerWS/) (WebSocket)
sketches. Camera status, camera controls and the JPEG video all travel
over one GATT connection using the shared Sensor Tester service
contract: the client writes the API key to the auth characteristic
before the node responds to anything.

**BLE is slow.** Depending on the negotiated MTU and connection
interval, throughput is roughly 20–100 KB/s, so the frame size is
**capped at VGA (640×480)** — larger `framesize` requests are clamped.
Expect a few frames per second at QVGA (the boot default) and around one
or two at VGA.

## GATT layout

Service `d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f`, advertised with the
device name `ESP32-CAM-BLE` (the app routes on this name).

| Characteristic | UUID suffix | Properties | Purpose |
| --- | --- | --- | --- |
| data | `-0002-` | READ, NOTIFY | Camera status JSON |
| auth | `-0003-` | WRITE | Shared API key |
| command | `-0004-` | WRITE, WRITE_NR | Binary commands |
| video | `-0005-` | NOTIFY | Chunked JPEG frames |

### Status (data characteristic)

The status JSON carries the same flat keys as the HTTP firmware's
`/status` endpoint (`framesize`, `quality`, `brightness`, ...,
`led_intensity`) plus `"chip"` (`OV2640`|`OV3660`|`OV5640`|`UNKNOWN`).
Notifications use the shared chunked framing from
[`../common/sensor_ble_framing.h`](../common/sensor_ble_framing.h)
(`0x1E <message-id> <chunk-index> <flags> <bytes>`); a direct read
returns the full JSON, or `{}` until authorized. Status is pushed after
a successful auth, after every applied control change, and on request.

### Commands (command characteristic)

| Packet | Effect |
| --- | --- |
| `0x01 <varId u8> <value s16 LE> [requestId u8]` | Set one control, acknowledge it, then push status. The request id is optional for older clients; `0` means no acknowledgement. |
| `0x02 <0\|1>` | Video off / on. |
| `0x03` | Request a status notification. |
| `0x04` | Send one still frame without turning the video on. |

Control variable ids (shared with the app):

| id | control | id | control |
| --- | --- | --- | --- |
| 1 | `special_effect` | 6 | `brightness` |
| 2 | `wb_mode` | 7 | `contrast` |
| 3 | `led_intensity` | 8 | `saturation` |
| 4 | `framesize` (≤ 10 = VGA) | 9 | `hmirror` |
| 5 | `quality` | 10 | `vflip` |

For a non-zero request id the data characteristic returns a framed JSON
acknowledgement before the refreshed status:

```json
{"ack":7,"ok":true,"var":6,"val":-1}
```

`val` is the value actually held by the camera. Invalid values and rejected
sensor operations return `"ok":false`; controls are range-checked in the
firmware rather than passed unchecked to the sensor/PWM driver.

### Video (video characteristic)

Each JPEG frame is chunked as

```
0x1F <frameId u8> <chunkIndex u16 LE> <flags u8> <payload>
```

with flags `0x01` = first chunk and `0x02` = last chunk. A chunk carries
up to `MTU − 3 − 5` payload bytes, so the client should negotiate a
large MTU (the node requests 517). A gap in the chunk sequence or a
changed `frameId` mid-frame means the frame must be dropped.

The flash LED (`led_intensity`, AI-Thinker GPIO 4) follows the
requested intensity whenever an authorized client is connected. Unlike
the HTTP/WS variants it is *not* tied to the video being on: the app's
BLE screen is still-first (video off by default), so gating the LED on
the stream would make the control a no-op. A still (`0x04`) is
therefore captured under whatever LED intensity is currently set, and
the LED turns off when the client disconnects.

A snapshot request remains pending if the camera cannot acquire a frame. The
node retries it from `loop()` instead of silently consuming the request; the
app independently times out a lost/incomplete BLE transfer so the capture
button can be used again.

## Build & flash

```bash
./install.sh /dev/cu.usbserial-0001
```

On the first run the script creates `secrets.h` from
`secrets.h.example`; fill in your API key (min. 8 characters, must match
the key configured in the Sensor Tester app), then re-run. No Wi-Fi
credentials are needed.

The script uses the AI-Thinker board definition
(`esp32:esp32:esp32cam`: PSRAM enabled, 3 MB app partition). For a
different ESP32 camera board, change the camera model in
`board_config.h` and use an FQBN with a large app partition, e.g.
`esp32:esp32:esp32:PartitionScheme=huge_app`.

Required library (installed by the script): `ArduinoJson`. BLE uses the
ESP32 core's built-in stack.
