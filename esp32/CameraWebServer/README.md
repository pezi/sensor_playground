# ESP32-CAM Camera Node for Sensor Tester

This Arduino project makes an ESP32 camera board discoverable by the
Sensor Tester app. It is based on Espressif's official
[CameraWebServer example](https://github.com/espressif/arduino-esp32/tree/master/libraries/ESP32/examples/Camera/CameraWebServer)
and extends it with the same UDP discovery mechanism used by the other
sensor nodes in this repository, so the app can find a camera on the
local network the same way it finds a sensor.

## Changes vs. the original example

- **UDP discovery (scan) support added:** the sketch listens on UDP port
  `9133` and answers `SENSOR_TESTER` broadcast probes (the Sensor Tester
  discovery contract, see the other `esp32_*` nodes). The reply is a
  short-key JSON telling the app where to reach the camera:

  ```json
  {"type":"ESP32-CAM","host":"ESP32-CAM","ip":"192.168.1.42","port":80,"stream_port":81,"chip":"OV2640"}
  ```

  `stream_port` and `chip` are extra keys beyond the standard contract:
  `stream_port` points to the MJPEG stream server, and `chip` names the
  detected camera sensor (`OV2640`, `OV3660` or `OV5640`; `UNKNOWN` if
  the sensor is not recognized). Clients that only read the standard
  keys (`type`, `host`, `ip`, `port`) are unaffected.
- The previously empty `loop()` now polls for discovery packets; the
  camera web server itself still runs in its own task, unchanged.
- The JSON reply is built with `snprintf`, so unlike the sensor nodes
  this sketch needs **no ArduinoJson library**.
- **WiFi credentials moved to a gitignored `secrets.h`** (same schema
  as the other nodes) instead of being hardcoded in the sketch.

Everything else (camera setup, web UI, streaming) is the stock
Espressif example.

## Hardware Requirements

- An ESP32 camera board, e.g. **AI-Thinker ESP32-CAM**, ESP-EYE,
  M5Stack camera modules, or ESP32-S3-EYE.
- Select your board by uncommenting the matching `CAMERA_MODEL_*`
  define in `board_config.h` (pin maps live in `camera_pins.h`).

## Software Requirements

1. **Arduino IDE** or **arduino-cli**
2. **Board Support**: ESP32 by Espressif
3. No additional libraries — the camera driver, WiFi and WiFiUdp all
   ship with the ESP32 core.

Boards without PSRAM work but are limited to smaller frame sizes; with
PSRAM the sketch pre-allocates larger frame buffers automatically.

## Configuration

Like the other sensor nodes, WiFi credentials live in a gitignored
`secrets.h`:

1. Copy `secrets.h.example` to `secrets.h`.
2. Enter your WiFi credentials (`WIFI_SSID`, `WIFI_PASS`) and
   optionally adjust the `HOSTNAME` shown in the app's scan list.

The camera serves plain HTTP without an API key, so unlike the sensor
sketches `secrets.h` needs no `API_KEY` and no TLS certificate. The
discovery `NODE_TYPE` stays in `CameraWebServer.ino` — the app routes
on it, so it should not be changed.

## How It Works

- **UDP Discovery (port 9133):** Responds to `SENSOR_TESTER` broadcasts
  with a JSON packet containing the node type, host name, IP, the two
  HTTP ports and the detected camera sensor chip.
- **Web UI / snapshot (port 80):** The stock camera web interface;
  `GET /capture` returns a single JPEG.
- **MJPEG stream (port 81):** `GET /stream` serves the continuous
  stream.

There is no API key or TLS on the camera endpoints — this is the plain
HTTP server from the upstream example.

## Testing

Probe the discovery port (replace the IP with the camera's):

```bash
echo -n "SENSOR_TESTER" | nc -u -w1 <esp32-cam-ip> 9133
```

Expected response (`chip` reflects the board's camera sensor):

```json
{"type":"ESP32-CAM","host":"ESP32-CAM","ip":"192.168.1.42","port":80,"stream_port":81,"chip":"OV2640"}
```

Then open `http://<esp32-cam-ip>/` for the web UI or
`http://<esp32-cam-ip>:81/stream` for the raw MJPEG stream.
