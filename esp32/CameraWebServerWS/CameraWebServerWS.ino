/*
 * Sensor Playground ESP32-CAM Node — WebSocket variant
 *
 * WebSocket-only counterpart of the plain-HTTP CameraWebServer sketch:
 * camera status, camera controls and the JPEG video all travel over a
 * single WebSocket server on port 9132, authenticated with the shared
 * X-Api-Key header at the upgrade handshake — the same contract every
 * other WebSocket node (LED, SSD1306, PAJ7620, ...) uses. There is no
 * HTTP server, no web UI and no snapshot endpoint.
 *
 * Protocol (ws://<ip>:9132):
 *
 *   app -> node (TEXT, JSON)
 *     {"status":true}                  send the status object to this client
 *     {"id":1,"var":"framesize","val":10}
 *                                      apply one control (same var names and
 *                                      value ranges as the HTTP /control
 *                                      endpoint), then broadcast the fresh
 *                                      status to every client
 *     {"stream":true|false}            per-client opt-in/out of video frames
 *
 *   node -> app
 *     TEXT    status object: the same flat keys the HTTP /status endpoint
 *             emits (framesize, quality, brightness, ..., led_intensity)
 *             plus "chip" ("OV2640"|"OV3660"|"OV5640"|"UNKNOWN"). Sent on
 *             connect, on {"status":true} and broadcast after every applied
 *             control change. The raw sensor-register dump of the HTTP
 *             firmware is not included — the explicit "chip" key replaces
 *             the register-based chip inference.
 *     BINARY  one complete JPEG image per frame (SOI..EOI), only to clients
 *             that opted in with {"stream":true}. No multipart framing.
 *     TEXT    command acknowledgement:
 *             {"ack":1,"ok":true,"var":"framesize","val":10}
 *
 * Discovery: UDP "SENSOR_TESTER" probe on port 9133, answered with
 * {"type":"ESP32-CAM-WS","host",...,"port":9132,"chip":"..."} — no
 * stream_port, since control and video share the WebSocket port.
 *
 * The flash LED (led_intensity) lights only while at least one client is
 * streaming, matching the HTTP firmware's behavior.
 *
 * Wi-Fi only: video streaming over BLE is infeasible, so unlike the sensor
 * sketches there is no ACTIVE_TRANSPORT / BLE build.
 *
 * Required Libraries: ArduinoJson, WebSockets (Markus Sattler / Links2004).
 */

#include <Arduino.h>
#include "esp_camera.h"
#include <WiFi.h>
#include "../common/sensor_wifi_runtime.h"
#include <WiFiUdp.h>
#include <WebSocketsServer.h>
#include <ArduinoJson.h>

// ===========================
// Select camera model in board_config.h
// ===========================
#include "board_config.h"

// ===========================
// WiFi credentials, API key and the node's host name live in secrets.h —
// copy secrets.h.example to secrets.h and fill in your values.
// ===========================
#include "secrets.h"

const char *NODE_TYPE = "ESP32-CAM-WS";
const int WS_PORT = 9132;
const int UDP_PORT = 9133;

// Pacing cap for the frame pump (~20 fps). Larger resolutions are slower
// anyway because sendBIN writes the whole JPEG synchronously.
const unsigned long FRAME_MIN_INTERVAL_MS = 50;

WiFiUDP udp;
WebSocketsServer webSocket = WebSocketsServer(WS_PORT);

// Per-client video opt-in, indexed by the library's client number.
bool streamOn[WEBSOCKETS_SERVER_CLIENT_MAX] = {false};
unsigned long lastFrameMs = 0;

#if defined(LED_GPIO_NUM)
// Flash LED duty requested via led_intensity (0-255). The LED only lights
// while a client is streaming, like the HTTP firmware.
int ledDuty = 0;
#endif

// Clients must present this header on the WebSocket handshake — unless
// API_KEY in secrets.h is empty (""): then the validator is never
// registered (see setup()) and every client connects, with or without the
// header. This keeps the sketch usable from apps without an API key
// concept (e.g. the ESP32-CAM app with an empty key setting).
const char *MANDATORY_HEADERS[] = {"X-Api-Key"};
const size_t MANDATORY_HEADER_COUNT = 1;

bool validateApiKey(String headerName, String headerValue) {
  if (strlen(API_KEY) == 0) {
    return true;  // key check disabled
  }
  if (headerName.equalsIgnoreCase("X-Api-Key")) {
    headerValue.trim();
    return headerValue == String(API_KEY);
  }
  return true;
}

// Name of the detected camera sensor chip, for discovery and status.
const char *chipName() {
  sensor_t *s = esp_camera_sensor_get();
  if (s == NULL) return "UNKNOWN";
  switch (s->id.PID) {
    case OV2640_PID: return "OV2640";
    case OV3660_PID: return "OV3660";
    case OV5640_PID: return "OV5640";
    default: return "UNKNOWN";
  }
}

bool anyStreaming() {
  for (uint8_t i = 0; i < WEBSOCKETS_SERVER_CLIENT_MAX; i++) {
    if (streamOn[i]) return true;
  }
  return false;
}

// Drive the flash LED from the requested duty and the streaming state.
void updateLed() {
#if defined(LED_GPIO_NUM)
  int duty = anyStreaming() ? ledDuty : 0;
  if (duty < 0) duty = 0;
  if (duty > 255) duty = 255;
  ledcWrite(LED_GPIO_NUM, duty);
#endif
}

void setupLedFlash() {
#if defined(LED_GPIO_NUM)
  ledcAttach(LED_GPIO_NUM, 5000, 8);
#endif
}

// Serializes the camera status: the same flat keys as the HTTP firmware's
// /status handler (minus the raw register dump) plus the chip name.
String statusJson() {
  StaticJsonDocument<1024> doc;
  sensor_t *s = esp_camera_sensor_get();
  if (s != NULL) {
    doc["xclk"] = s->xclk_freq_hz / 1000000;
    doc["pixformat"] = (int)s->pixformat;
    doc["framesize"] = (int)s->status.framesize;
    doc["quality"] = s->status.quality;
    doc["brightness"] = s->status.brightness;
    doc["contrast"] = s->status.contrast;
    doc["saturation"] = s->status.saturation;
    doc["sharpness"] = s->status.sharpness;
    doc["special_effect"] = s->status.special_effect;
    doc["wb_mode"] = s->status.wb_mode;
    doc["awb"] = s->status.awb;
    doc["awb_gain"] = s->status.awb_gain;
    doc["aec"] = s->status.aec;
    doc["aec2"] = s->status.aec2;
    doc["ae_level"] = s->status.ae_level;
    doc["aec_value"] = s->status.aec_value;
    doc["agc"] = s->status.agc;
    doc["agc_gain"] = s->status.agc_gain;
    doc["gainceiling"] = (int)s->status.gainceiling;
    doc["bpc"] = s->status.bpc;
    doc["wpc"] = s->status.wpc;
    doc["raw_gma"] = s->status.raw_gma;
    doc["lenc"] = s->status.lenc;
    doc["hmirror"] = s->status.hmirror;
    doc["vflip"] = s->status.vflip;
    doc["dcw"] = s->status.dcw;
    doc["colorbar"] = s->status.colorbar;
  }
#if defined(LED_GPIO_NUM)
  doc["led_intensity"] = ledDuty;
#else
  doc["led_intensity"] = -1;
#endif
  doc["chip"] = chipName();
  String out;
  serializeJson(doc, out);
  return out;
}

// Reject malformed/out-of-range values at the transport boundary. Sensor
// setters still make the final chip-specific capability decision.
bool validControlValue(const char *variable, int val) {
  if (!strcmp(variable, "framesize")) return val >= 0 && val < FRAMESIZE_INVALID;
  if (!strcmp(variable, "quality")) return val >= 0 && val <= 63;
  if (!strcmp(variable, "contrast") || !strcmp(variable, "brightness") ||
      !strcmp(variable, "saturation") || !strcmp(variable, "ae_level")) {
    return val >= -2 && val <= 2;
  }
  if (!strcmp(variable, "gainceiling")) return val >= 0 && val <= 6;
  if (!strcmp(variable, "agc_gain")) return val >= 0 && val <= 30;
  if (!strcmp(variable, "aec_value")) return val >= 0 && val <= 1200;
  if (!strcmp(variable, "special_effect")) return val >= 0 && val <= 6;
  if (!strcmp(variable, "wb_mode")) return val >= 0 && val <= 4;
  if (!strcmp(variable, "led_intensity")) return val >= 0 && val <= 255;
  if (!strcmp(variable, "colorbar") || !strcmp(variable, "awb") ||
      !strcmp(variable, "agc") || !strcmp(variable, "aec") ||
      !strcmp(variable, "hmirror") || !strcmp(variable, "vflip") ||
      !strcmp(variable, "awb_gain") || !strcmp(variable, "aec2") ||
      !strcmp(variable, "dcw") || !strcmp(variable, "bpc") ||
      !strcmp(variable, "wpc") || !strcmp(variable, "raw_gma") ||
      !strcmp(variable, "lenc")) {
    return val == 0 || val == 1;
  }
  return false;
}

// Returns the actual value for the controls surfaced by the Flutter app.
bool controlValue(const char *variable, int *value) {
  sensor_t *s = esp_camera_sensor_get();
  if (s == NULL || value == NULL) return false;
  if (!strcmp(variable, "framesize")) *value = (int)s->status.framesize;
  else if (!strcmp(variable, "quality")) *value = s->status.quality;
  else if (!strcmp(variable, "contrast")) *value = s->status.contrast;
  else if (!strcmp(variable, "brightness")) *value = s->status.brightness;
  else if (!strcmp(variable, "saturation")) *value = s->status.saturation;
  else if (!strcmp(variable, "hmirror")) *value = s->status.hmirror;
  else if (!strcmp(variable, "vflip")) *value = s->status.vflip;
  else if (!strcmp(variable, "special_effect")) *value = s->status.special_effect;
  else if (!strcmp(variable, "wb_mode")) *value = s->status.wb_mode;
#if defined(LED_GPIO_NUM)
  else if (!strcmp(variable, "led_intensity")) *value = ledDuty;
#endif
  else return false;
  return true;
}

// Applies one validated control, mirroring the HTTP firmware's cmd_handler.
// Returns false for an unknown variable or a rejected value.
bool applyControl(const char *variable, int val) {
  if (!validControlValue(variable, val)) return false;
  sensor_t *s = esp_camera_sensor_get();
  if (s == NULL) return false;
  int res = 0;

  if (!strcmp(variable, "framesize")) {
    if (s->pixformat == PIXFORMAT_JPEG) {
      res = s->set_framesize(s, (framesize_t)val);
    }
  } else if (!strcmp(variable, "quality")) {
    res = s->set_quality(s, val);
  } else if (!strcmp(variable, "contrast")) {
    res = s->set_contrast(s, val);
  } else if (!strcmp(variable, "brightness")) {
    res = s->set_brightness(s, val);
  } else if (!strcmp(variable, "saturation")) {
    res = s->set_saturation(s, val);
  } else if (!strcmp(variable, "gainceiling")) {
    res = s->set_gainceiling(s, (gainceiling_t)val);
  } else if (!strcmp(variable, "colorbar")) {
    res = s->set_colorbar(s, val);
  } else if (!strcmp(variable, "awb")) {
    res = s->set_whitebal(s, val);
  } else if (!strcmp(variable, "agc")) {
    res = s->set_gain_ctrl(s, val);
  } else if (!strcmp(variable, "aec")) {
    res = s->set_exposure_ctrl(s, val);
  } else if (!strcmp(variable, "hmirror")) {
    res = s->set_hmirror(s, val);
  } else if (!strcmp(variable, "vflip")) {
    res = s->set_vflip(s, val);
  } else if (!strcmp(variable, "awb_gain")) {
    res = s->set_awb_gain(s, val);
  } else if (!strcmp(variable, "agc_gain")) {
    res = s->set_agc_gain(s, val);
  } else if (!strcmp(variable, "aec_value")) {
    res = s->set_aec_value(s, val);
  } else if (!strcmp(variable, "aec2")) {
    res = s->set_aec2(s, val);
  } else if (!strcmp(variable, "dcw")) {
    res = s->set_dcw(s, val);
  } else if (!strcmp(variable, "bpc")) {
    res = s->set_bpc(s, val);
  } else if (!strcmp(variable, "wpc")) {
    res = s->set_wpc(s, val);
  } else if (!strcmp(variable, "raw_gma")) {
    res = s->set_raw_gma(s, val);
  } else if (!strcmp(variable, "lenc")) {
    res = s->set_lenc(s, val);
  } else if (!strcmp(variable, "special_effect")) {
    res = s->set_special_effect(s, val);
  } else if (!strcmp(variable, "wb_mode")) {
    res = s->set_wb_mode(s, val);
  } else if (!strcmp(variable, "ae_level")) {
    res = s->set_ae_level(s, val);
  }
#if defined(LED_GPIO_NUM)
  else if (!strcmp(variable, "led_intensity")) {
    ledDuty = val;
    updateLed();
  }
#endif
  else {
    Serial.printf("Unknown control: %s\n", variable);
    return false;
  }
  return res >= 0;
}

void sendControlResult(uint8_t num, int requestId, const char *variable,
                       bool ok) {
  if (requestId <= 0) return;
  StaticJsonDocument<160> result;
  result["ack"] = requestId;
  result["ok"] = ok;
  result["var"] = variable;
  int actual = 0;
  if (controlValue(variable, &actual)) result["val"] = actual;
  String out;
  serializeJson(result, out);
  webSocket.sendTXT(num, out);
}

// Execute one JSON command pushed by the app.
void handleCommand(uint8_t num, uint8_t *payload, size_t len) {
  StaticJsonDocument<192> doc;
  // Zero-copy parse; the WebSockets library null-terminates text frames.
  DeserializationError error = deserializeJson(doc, (char *)payload, len);
  if (error) {
    Serial.println("Ignoring malformed command");
    return;
  }

  if (doc["stream"].is<bool>()) {
    bool on = doc["stream"];
    Serial.printf("[%u] Command: stream %s\n", num, on ? "on" : "off");
    if (num < WEBSOCKETS_SERVER_CLIENT_MAX) streamOn[num] = on;
    updateLed();
    return;
  }

  if (doc["status"] == true) {
    String out = statusJson();
    webSocket.sendTXT(num, out);
    return;
  }

  if (doc["var"].is<const char *>() && doc["val"].is<int>()) {
    const char *variable = doc["var"];
    int val = doc["val"];
    int requestId = doc["id"].is<int>() ? doc["id"].as<int>() : 0;
    Serial.printf("[%u] Command: %s = %d\n", num, variable, val);
    bool applied = applyControl(variable, val);
    // Broadcast the resulting status so every client tracks the actual
    // camera state, whether the value was accepted or clamped.
    String out = statusJson();
    webSocket.broadcastTXT(out);
    sendControlResult(num, requestId, variable, applied);
    return;
  }

  Serial.println("Ignoring unknown command");
}

void webSocketEvent(uint8_t num, WStype_t type, uint8_t *payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED: {
      Serial.printf("[%u] Client connected\n", num);
      // Send the current status so the app can render controls right away.
      // sendTXT takes String& (an lvalue), so hold it in a variable.
      String out = statusJson();
      webSocket.sendTXT(num, out);
      break;
    }
    case WStype_DISCONNECTED:
      Serial.printf("[%u] Client disconnected\n", num);
      if (num < WEBSOCKETS_SERVER_CLIENT_MAX) streamOn[num] = false;
      updateLed();
      break;
    case WStype_TEXT:
      handleCommand(num, payload, len);
      break;
    default:
      break;
  }
}

void handleUdpDiscovery() {
  int packetSize = udp.parsePacket();
  if (!packetSize) return;

  char buffer[64];
  int len = udp.read(buffer, sizeof(buffer) - 1);
  if (len < 0) len = 0;
  buffer[len] = '\0';
  if (strstr(buffer, "SENSOR_TESTER") == NULL) return;

  // A camera has no readings to advertise — identity plus the chip name,
  // which spares the app a probe over the data channel.
  StaticJsonDocument<256> doc;
  doc["type"] = NODE_TYPE;
  doc["host"] = HOSTNAME;
  doc["ip"] = WiFi.localIP().toString();
  doc["port"] = WS_PORT;
  doc["chip"] = chipName();
  String json;
  serializeJson(doc, json);

  udp.beginPacket(udp.remoteIP(), udp.remotePort());
  udp.print(json);
  udp.endPacket();
}

// Grab one frame and push it to every streaming client.
void pumpFrame() {
  if (!anyStreaming()) return;
  if (millis() - lastFrameMs < FRAME_MIN_INTERVAL_MS) return;

  camera_fb_t *fb = esp_camera_fb_get();
  if (!fb) return;
  // pixel_format is PIXFORMAT_JPEG, so fb->buf is a complete JPEG image.
  for (uint8_t i = 0; i < WEBSOCKETS_SERVER_CLIENT_MAX; i++) {
    if (streamOn[i] && webSocket.clientIsConnected(i)) {
      webSocket.sendBIN(i, fb->buf, fb->len);
    }
  }
  esp_camera_fb_return(fb);
  lastFrameMs = millis();
}

void setup() {
  Serial.begin(115200);
  Serial.setDebugOutput(true);
  Serial.println();

  camera_config_t config;
  config.ledc_channel = LEDC_CHANNEL_0;
  config.ledc_timer = LEDC_TIMER_0;
  config.pin_d0 = Y2_GPIO_NUM;
  config.pin_d1 = Y3_GPIO_NUM;
  config.pin_d2 = Y4_GPIO_NUM;
  config.pin_d3 = Y5_GPIO_NUM;
  config.pin_d4 = Y6_GPIO_NUM;
  config.pin_d5 = Y7_GPIO_NUM;
  config.pin_d6 = Y8_GPIO_NUM;
  config.pin_d7 = Y9_GPIO_NUM;
  config.pin_xclk = XCLK_GPIO_NUM;
  config.pin_pclk = PCLK_GPIO_NUM;
  config.pin_vsync = VSYNC_GPIO_NUM;
  config.pin_href = HREF_GPIO_NUM;
  config.pin_sccb_sda = SIOD_GPIO_NUM;
  config.pin_sccb_scl = SIOC_GPIO_NUM;
  config.pin_pwdn = PWDN_GPIO_NUM;
  config.pin_reset = RESET_GPIO_NUM;
  config.xclk_freq_hz = 20000000;
  config.frame_size = FRAMESIZE_UXGA;
  config.pixel_format = PIXFORMAT_JPEG;  // for streaming
  config.grab_mode = CAMERA_GRAB_WHEN_EMPTY;
  config.fb_location = CAMERA_FB_IN_PSRAM;
  config.jpeg_quality = 12;
  config.fb_count = 1;

  // if PSRAM IC present, init with UXGA resolution and higher JPEG quality
  //                      for larger pre-allocated frame buffer.
  if (psramFound()) {
    config.jpeg_quality = 10;
    config.fb_count = 2;
    config.grab_mode = CAMERA_GRAB_LATEST;
  } else {
    // Limit the frame size when PSRAM is not available
    config.frame_size = FRAMESIZE_SVGA;
    config.fb_location = CAMERA_FB_IN_DRAM;
  }

#if defined(CAMERA_MODEL_ESP_EYE)
  pinMode(13, INPUT_PULLUP);
  pinMode(14, INPUT_PULLUP);
#endif

  // camera init
  esp_err_t err = esp_camera_init(&config);
  if (err != ESP_OK) {
    Serial.printf("Camera init failed with error 0x%x", err);
    return;
  }

  sensor_t *s = esp_camera_sensor_get();
  // initial sensors are flipped vertically and colors are a bit saturated
  if (s->id.PID == OV3660_PID) {
    s->set_vflip(s, 1);        // flip it back
    s->set_brightness(s, 1);   // up the brightness just a bit
    s->set_saturation(s, -2);  // lower the saturation
  }
  // drop down frame size for higher initial frame rate
  s->set_framesize(s, FRAMESIZE_QVGA);

#if defined(CAMERA_MODEL_M5STACK_WIDE) || defined(CAMERA_MODEL_M5STACK_ESP32CAM)
  s->set_vflip(s, 1);
  s->set_hmirror(s, 1);
#endif

#if defined(CAMERA_MODEL_ESP32S3_EYE)
  s->set_vflip(s, 1);
#endif

#if defined(LED_GPIO_NUM)
  setupLedFlash();
#endif

  if (!connectSensorWifi(WIFI_SSID, WIFI_PASS)) {
    Serial.println("Restarting after WiFi setup failure");
    delay(1000);
    ESP.restart();
  }

  webSocket.begin();
  webSocket.onEvent(webSocketEvent);
  // An empty API_KEY disables the key check: registering the validator
  // would still make the X-Api-Key header mandatory, so skip it entirely.
  if (strlen(API_KEY) > 0) {
    webSocket.onValidateHttpHeader(validateApiKey, MANDATORY_HEADERS,
                                   MANDATORY_HEADER_COUNT);
  } else {
    Serial.println("API key check disabled (empty API_KEY)");
  }

  udp.begin(UDP_PORT);
  Serial.printf("WebSocket on port %d, UDP discovery on port %d\n",
                WS_PORT, UDP_PORT);
  Serial.print("Camera Ready! Connect to 'ws://");
  Serial.print(WiFi.localIP());
  Serial.printf(":%d'\n", WS_PORT);
}

void loop() {
  if (!sensorWifiReady()) {
    delay(10);
    return;
  }
  webSocket.loop();
  handleUdpDiscovery();
  pumpFrame();
  delay(1);
}
