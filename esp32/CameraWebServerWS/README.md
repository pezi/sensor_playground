# CameraWebServerWS — ESP32-CAM WebSocket Node

This sketch is used by the following apps:


**Sensor Playground**

[![Get it on Google Play](https://img.shields.io/badge/Google_Play-Download-414141?logo=google-play&logoColor=white)](https://play.google.com/store/apps/details?id=app.flutterdev.sensortester)
[![Get it on the App Store](https://img.shields.io/badge/App_Store-Download-0D96F6?style=flat&logo=app-store&logoColor=white)](https://apps.apple.com/us/app/sensor-playground/id6778514035)

**ESP32-Cam** 

[![Get it on Google Play](https://img.shields.io/badge/Google_Play-Download-414141?logo=google-play&logoColor=white)](https://play.google.com/store/apps/details?id=at.flutterdev.esp32_cam)
[![Get it on the App Store](https://img.shields.io/badge/App_Store-Download-0D96F6?style=flat&logo=app-store&logoColor=white)](https://apps.apple.com/us/app/sensor-playground/id6754187049)



WebSocket-only variant of the [`CameraWebServer`](../CameraWebServer/)
sketch. Camera status, camera controls and the JPEG video all travel over
a single WebSocket server, authenticated with the shared `X-Api-Key`
header — the same contract every other WebSocket node (LED, SSD1306,
PAJ7620, VL53L0X, ...) uses. There is no HTTP server, no web UI and no
snapshot endpoint, and unlike the plain-HTTP camera this node is **not**
unauthenticated.

The node is Wi-Fi only: video streaming over BLE is infeasible, so there
is no `ACTIVE_TRANSPORT` / BLE build.

## Ports

| Port | Purpose |
| --- | --- |
| 9132 | WebSocket (`ws://<ip>:9132`) — control, status and video |
| 9133 | UDP discovery (`SENSOR_TESTER` probe) |

Discovery reply:

```json
{"type":"ESP32-CAM-WS","host":"ESP32-CAM-WS","ip":"192.168.0.50","port":9132,"chip":"OV5640"}
```

There is no `stream_port` key — control and video share the WebSocket
port. The `chip` key names the detected sensor (`OV2640`, `OV3660`,
`OV5640` or `UNKNOWN`) so the app never needs to probe for it.

## WebSocket protocol

Clients must send the shared API key in the `X-Api-Key` header of the
WebSocket upgrade request; the handshake is rejected otherwise. Setting
`API_KEY` to the empty string (`""`) in `secrets.h` disables the check
entirely — every client connects, with or without the header. Use this
for clients without an API key concept, e.g. the ESP32-CAM app with an
empty API key setting.

### App → node (TEXT frames, JSON)

| Message | Effect |
| --- | --- |
| `{"status":true}` | Send the status object to this client. |
| `{"id":1,"var":"framesize","val":10}` | Apply one range-checked control, broadcast fresh status, and acknowledge the request with the actual value. `id` may be omitted by older clients. |
| `{"stream":true}` / `{"stream":false}` | Per-client opt-in/out of binary video frames. |

Malformed or unknown messages are logged and ignored.

### Node → app

- **Status (TEXT frame):** sent on connect, on `{"status":true}` and
  broadcast after every applied control change. A flat JSON object with
  the same keys the HTTP firmware's `/status` endpoint emits
  (`framesize`, `quality`, `brightness`, ..., `led_intensity`) plus
  `"chip"`. The raw sensor-register dump of the HTTP firmware is not
  included; the explicit `chip` key replaces the register-based chip
  inference.
- **Video (BINARY frames):** one complete JPEG image (SOI..EOI) per
  frame, only to clients that opted in with `{"stream":true}`. No
  multipart framing, no boundaries. The opt-in flag is cleared when the
  client disconnects.
- **Control acknowledgement (TEXT frame):**
  `{"ack":1,"ok":true,"var":"framesize","val":10}` is sent to the
  requesting client after a command. `val` is the value actually held by the
  camera. Invalid values or rejected sensor operations use `"ok":false`.

The Flutter client waits for this acknowledgement before changing its
confirmed settings. If the control socket closes after startup, the camera
screen enters its disconnected/retry state even if the separate video socket
has not failed yet.

The flash LED (`led_intensity`, AI-Thinker GPIO 4) lights only while at
least one client is streaming, matching the HTTP firmware's behavior.

## Performance notes

- Frames are pumped at most every 50 ms (~20 fps); the actual rate at
  higher resolutions is limited by the synchronous WebSocket write.
- Frames are written synchronously from `loop()`, so very large frames
  (e.g. OV5640 at QSXGA) delay control handling — prefer UXGA or below
  for fluid control response.
- In practice one client should consume the video at a time; a stalled
  consumer can hold up the frame pump until its TCP connection times out.

## Build & flash

```bash
./install.sh /dev/cu.usbserial-0001
```

On the first run the script creates `secrets.h` from
`secrets.h.example`; fill in your Wi-Fi credentials, API key (min. 8
characters, must match the key configured in the Sensor Playground /
ESP32-CAM app — or empty to disable the key check) and host name, then
re-run.

The script uses the AI-Thinker board definition
(`esp32:esp32:esp32cam`: PSRAM enabled, 3 MB app partition). For a
different ESP32 camera board, change the camera model in
`board_config.h` and use an FQBN with a large app partition, e.g.
`esp32:esp32:esp32:PartitionScheme=huge_app`.

Required libraries (installed by the script): `ArduinoJson`,
`WebSockets` (Markus Sattler / Links2004).
