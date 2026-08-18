/*
 * Sensor Tester ESP32-CAM Node — BLE variant
 *
 * BLE-only counterpart of the CameraWebServer (HTTP) and CameraWebServerWS
 * (WebSocket) sketches: camera status, camera controls and the JPEG video
 * all travel over one GATT connection, using the shared Sensor Tester
 * service contract (write the API key to the auth characteristic first).
 *
 * BLE is slow compared to Wi-Fi — roughly 20-100 KB/s depending on the
 * negotiated MTU and connection interval — so the frame size is capped at
 * VGA (640x480, framesize 10). Expect a few frames per second at QVGA and
 * around one or two at VGA.
 *
 * GATT layout (matching the app's BleUuids):
 *   d1a51b00-0001-...  service (advertised, device name "ESP32-CAM-BLE")
 *   d1a51b00-0002-...  data:   READ | NOTIFY — camera status JSON. Notifies
 *                      use the shared 0x1E chunked framing
 *                      (../common/sensor_ble_framing.h); a direct read
 *                      returns the full JSON ("{}" until authorized).
 *                      Status is pushed after auth, after every applied
 *                      control change and on request. The JSON carries the
 *                      same flat keys as the HTTP /status endpoint plus
 *                      "chip" ("OV2640"|"OV3660"|"OV5640"|"UNKNOWN").
 *   d1a51b00-0003-...  auth:   WRITE — the shared API key.
 *   d1a51b00-0004-...  command: WRITE | WRITE_NR — binary commands:
 *                        0x01 <varId u8> <value s16 LE> [requestId u8]
 *                                                        set one control
 *                        0x02 <0|1>                      video off / on
 *                        0x03                            request a status
 *                        0x04                            send one still frame
 *                      Control variable ids (shared with the app):
 *                        1 special_effect  2 wb_mode   3 led_intensity
 *                        4 framesize       5 quality   6 brightness
 *                        7 contrast        8 saturation
 *                        9 hmirror        10 vflip
 *   d1a51b00-0005-...  video:  NOTIFY — JPEG frames, chunked as
 *                        0x1F <frameId u8> <chunkIndex u16 LE> <flags u8>
 *                        <payload>
 *                      flags: 0x01 first chunk, 0x02 last chunk. Each
 *                      chunk carries up to MTU-3-5 payload bytes, so the
 *                      app should negotiate a large MTU. A gap in the
 *                      chunk sequence means the frame must be dropped.
 *
 * The flash LED (led_intensity) follows the requested intensity whenever
 * an authorized client is connected. Unlike the HTTP/WS variants it is
 * NOT tied to the video being on: the app's BLE screen is still-first
 * (video off by default), so gating on the stream would make the LED
 * control a no-op. It turns off on disconnect.
 *
 * Required Libraries: ArduinoJson. (BLE uses the ESP32 core's built-in
 * BLE stack.)
 */

#include <Arduino.h>
#include "esp_camera.h"
#include <ArduinoJson.h>
#include <BLEDevice.h>
#include <BLEServer.h>
#include <BLEUtils.h>
#include <BLE2902.h>
#include "../common/sensor_ble_framing.h"

// ===========================
// Select camera model in board_config.h
// ===========================
#include "board_config.h"

// ===========================
// The API key lives in secrets.h — copy secrets.h.example to secrets.h
// and fill in your value.
// ===========================
#include "secrets.h"

// Shared Sensor Tester GATT contract (must match the app's BleUuids).
#define SERVICE_UUID    "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
#define DATA_CHAR_UUID  "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
#define AUTH_CHAR_UUID  "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
#define CMD_CHAR_UUID   "d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f"
#define VIDEO_CHAR_UUID "d1a51b00-0005-4a7e-9b3c-0a1b2c3d4e5f"

// Advertised device name; the app routes on it like a discovery `type`.
const char* SENSOR_NAME = "ESP32-CAM-BLE";

// Command opcodes (see the header comment).
const uint8_t CMD_SET_CONTROL = 0x01;
const uint8_t CMD_STREAM = 0x02;
const uint8_t CMD_STATUS = 0x03;
const uint8_t CMD_SNAPSHOT = 0x04;

// Control variable ids (shared with the app's CameraCommand.bleId).
const uint8_t VAR_SPECIAL_EFFECT = 1;
const uint8_t VAR_WB_MODE = 2;
const uint8_t VAR_LED_INTENSITY = 3;
const uint8_t VAR_FRAMESIZE = 4;
const uint8_t VAR_QUALITY = 5;
const uint8_t VAR_BRIGHTNESS = 6;
const uint8_t VAR_CONTRAST = 7;
const uint8_t VAR_SATURATION = 8;
const uint8_t VAR_HMIRROR = 9;
const uint8_t VAR_VFLIP = 10;

// Video chunk framing.
const uint8_t VIDEO_FRAME_MARKER = 0x1F;
const uint8_t VIDEO_FRAME_START = 0x01;
const uint8_t VIDEO_FRAME_END = 0x02;
const size_t VIDEO_FRAME_HEADER = 5;

// BLE cannot move more than VGA at a usable rate.
const int MAX_FRAMESIZE = FRAMESIZE_VGA;  // 10 — 640x480

// Minimum pause between frames; the chunk loop dominates the real rate.
const unsigned long FRAME_MIN_INTERVAL_MS = 100;

// Pause between video chunk notifications, so the controller queues one
// before its characteristic value is replaced by the next.
const unsigned long CHUNK_DELAY_MS = 4;

BLECharacteristic* dataChar = nullptr;
BLECharacteristic* videoChar = nullptr;
bool deviceConnected = false;

// An empty API_KEY in secrets.h disables the key check entirely: clients
// are authorized without writing the auth characteristic. This keeps the
// sketch usable from apps without an API key concept (e.g. the ESP32-CAM
// app with an empty key setting). A client that still writes a key (the
// Sensor Playground app always does) stays authorized too.
static bool apiKeyDisabled() { return API_KEY[0] == '\0'; }

bool authed = false;  // reset to apiKeyDisabled() in setup()/onDisconnect
bool streamOn = false;

// One still frame was requested (CMD_SNAPSHOT); served by the next
// pumpFrame() pass without turning the continuous video on.
bool snapshotPending = false;

// ATT MTU in force, updated by onMtuChanged; 23 is the mandatory minimum.
uint16_t gMtu = 23;
uint8_t gFrameId = 0;
unsigned long lastFrameMs = 0;

#if defined(LED_GPIO_NUM)
// Flash LED duty requested via led_intensity (0-255). The LED lights
// whenever an authorized client is connected — the still-first BLE
// screen toggles it with the video off, so it must not gate on streamOn.
int ledDuty = 0;
#endif

// Name of the detected camera sensor chip, for the status JSON.
const char* chipName() {
  sensor_t* s = esp_camera_sensor_get();
  if (s == NULL) return "UNKNOWN";
  switch (s->id.PID) {
    case OV2640_PID: return "OV2640";
    case OV3660_PID: return "OV3660";
    case OV5640_PID: return "OV5640";
    default: return "UNKNOWN";
  }
}

// Drive the flash LED from the requested duty and the streaming state.
void updateLed() {
#if defined(LED_GPIO_NUM)
  int duty = (deviceConnected && authed) ? ledDuty : 0;
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
  sensor_t* s = esp_camera_sensor_get();
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

// Push the current status to an authorized client over the data
// characteristic, using the shared chunked notify framing.
void notifyStatus() {
  if (!deviceConnected || !authed) return;
  notifySensorJson(dataChar, statusJson());
}

// Reject malformed/out-of-range values at the transport boundary. Sensor
// setters still make the final chip-specific capability decision.
bool validControlValue(uint8_t varId, int val) {
  switch (varId) {
    case VAR_SPECIAL_EFFECT: return val >= 0 && val <= 6;
    case VAR_WB_MODE: return val >= 0 && val <= 4;
    case VAR_LED_INTENSITY: return val >= 0 && val <= 255;
    case VAR_FRAMESIZE: return val >= 0 && val < FRAMESIZE_INVALID;
    case VAR_QUALITY: return val >= 0 && val <= 63;
    case VAR_BRIGHTNESS:
    case VAR_CONTRAST:
    case VAR_SATURATION: return val >= -2 && val <= 2;
    case VAR_HMIRROR:
    case VAR_VFLIP: return val == 0 || val == 1;
    default: return false;
  }
}

// Returns the value the camera actually holds after a setter accepted or
// clamped a request. Used by command acknowledgements.
bool controlValue(uint8_t varId, int* value) {
  sensor_t* s = esp_camera_sensor_get();
  if (s == NULL || value == NULL) return false;
  switch (varId) {
    case VAR_SPECIAL_EFFECT: *value = s->status.special_effect; return true;
    case VAR_WB_MODE: *value = s->status.wb_mode; return true;
#if defined(LED_GPIO_NUM)
    case VAR_LED_INTENSITY: *value = ledDuty; return true;
#endif
    case VAR_FRAMESIZE: *value = (int)s->status.framesize; return true;
    case VAR_QUALITY: *value = s->status.quality; return true;
    case VAR_BRIGHTNESS: *value = s->status.brightness; return true;
    case VAR_CONTRAST: *value = s->status.contrast; return true;
    case VAR_SATURATION: *value = s->status.saturation; return true;
    case VAR_HMIRROR: *value = s->status.hmirror; return true;
    case VAR_VFLIP: *value = s->status.vflip; return true;
    default: return false;
  }
}

// Applies one control by its shared variable id. The frame size is capped at
// VGA — BLE cannot carry more — and larger valid requests are clamped.
bool applyControl(uint8_t varId, int val) {
  if (!validControlValue(varId, val)) return false;
  sensor_t* s = esp_camera_sensor_get();
  if (s == NULL) return false;
  int res = 0;

  switch (varId) {
    case VAR_SPECIAL_EFFECT:
      res = s->set_special_effect(s, val);
      break;
    case VAR_WB_MODE:
      res = s->set_wb_mode(s, val);
      break;
#if defined(LED_GPIO_NUM)
    case VAR_LED_INTENSITY:
      ledDuty = val;
      updateLed();
      break;
#endif
    case VAR_FRAMESIZE:
      if (val > MAX_FRAMESIZE) val = MAX_FRAMESIZE;
      if (s->pixformat == PIXFORMAT_JPEG) {
        res = s->set_framesize(s, (framesize_t)val);
      }
      break;
    case VAR_QUALITY:
      res = s->set_quality(s, val);
      break;
    case VAR_BRIGHTNESS:
      res = s->set_brightness(s, val);
      break;
    case VAR_CONTRAST:
      res = s->set_contrast(s, val);
      break;
    case VAR_SATURATION:
      res = s->set_saturation(s, val);
      break;
    case VAR_HMIRROR:
      res = s->set_hmirror(s, val);
      break;
    case VAR_VFLIP:
      res = s->set_vflip(s, val);
      break;
    default:
      Serial.printf("Unknown control id %u\n", varId);
      return false;
  }
  return res >= 0;
}

// Confirm one request with the accepted/rejected result and, where available,
// the actual value. This small JSON uses the normal reliable reassembler in
// the app and leaves status notifications backward compatible.
void notifyControlResult(uint8_t requestId, uint8_t varId, bool ok) {
  if (requestId == 0 || !deviceConnected || !authed) return;
  StaticJsonDocument<128> doc;
  doc["ack"] = requestId;
  doc["ok"] = ok;
  doc["var"] = varId;
  int actual = 0;
  if (controlValue(varId, &actual)) doc["val"] = actual;
  String out;
  serializeJson(doc, out);
  notifySensorJson(dataChar, out);
}

// Handle one command packet written to CMD_CHAR_UUID.
void handleBleCommand(const uint8_t* packet, size_t len) {
  if (len == 0) {
    Serial.println("Ignoring empty command packet");
    return;
  }

  switch (packet[0]) {
    case CMD_SET_CONTROL: {
      if (len < 4) {
        Serial.println("Ignoring short set-control packet");
        return;
      }
      uint8_t varId = packet[1];
      int val = (int16_t)(packet[2] | (packet[3] << 8));
      uint8_t requestId = len >= 5 ? packet[4] : 0;
      Serial.printf("Command: control %u = %d\n", varId, val);
      bool applied = applyControl(varId, val);
      notifyControlResult(requestId, varId, applied);
      // Push the resulting status so the app tracks the actual camera
      // state, whether the value was accepted or clamped.
      notifyStatus();
      return;
    }
    case CMD_STREAM:
      if (len < 2) {
        Serial.println("Ignoring stream command without a state byte");
        return;
      }
      streamOn = packet[1] != 0;
      Serial.printf("Command: stream %s\n", streamOn ? "on" : "off");
      lastFrameMs = 0;
      return;
    case CMD_STATUS:
      Serial.println("Command: status");
      notifyStatus();
      return;
    case CMD_SNAPSHOT:
      Serial.println("Command: snapshot");
      snapshotPending = true;
      return;
    default:
      Serial.printf("Ignoring unknown opcode 0x%02X\n", packet[0]);
      return;
  }
}

class ServerCallbacks : public BLEServerCallbacks {
  void onConnect(BLEServer* server) override { deviceConnected = true; }
  void onDisconnect(BLEServer* server) override {
    deviceConnected = false;
    authed = apiKeyDisabled();
    streamOn = false;
    snapshotPending = false;
    gMtu = 23;
    updateLed();
    server->getAdvertising()->start();  // allow the next client to find us
  }
  void onMtuChanged(BLEServer* server,
                    esp_ble_gatts_cb_param_t* param) override {
    // The video chunk size follows the negotiated MTU; without this the
    // stream would crawl at the 23-byte minimum.
    gMtu = param->mtu.mtu;
    Serial.printf("MTU changed to %u\n", gMtu);
  }
};

// Client must write the shared API key here before the camera responds.
class AuthCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    String val = characteristic->getValue();
    while (val.length() > 0 && (val[val.length() - 1] == '\0' ||
                                val[val.length() - 1] == '\r' ||
                                val[val.length() - 1] == '\n')) {
      val.remove(val.length() - 1);
    }
    authed = apiKeyDisabled() || (val == API_KEY);
    Serial.println(authed ? "Client authorized" : "Bad API key");
    // Push the current status right after a successful authorization, so
    // the app can render its controls without an extra request.
    if (authed) {
      notifyStatus();
    } else {
      streamOn = false;
      snapshotPending = false;
    }
    updateLed();
  }
};

// Serve the current status only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? statusJson().c_str() : "{}");
  }
};

class CommandCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    if (!authed) return;  // commands are gated like the sensor nodes' reads
    handleBleCommand(characteristic->getData(), characteristic->getLength());
  }
};

// Send one JPEG frame over the video characteristic in MTU-sized chunks.
void sendVideoFrame(const uint8_t* data, size_t length) {
  // A notification carries at most MTU-3 bytes; 5 of those are the chunk
  // header. Cap at the 517-byte maximum a peer may negotiate.
  uint16_t mtu = gMtu;
  if (mtu > 517) mtu = 517;
  if (mtu < 23) mtu = 23;
  const size_t chunkContent = (size_t)mtu - 3 - VIDEO_FRAME_HEADER;

  static uint8_t packet[517];
  gFrameId++;
  uint16_t chunkIndex = 0;

  for (size_t offset = 0; offset < length;
       offset += chunkContent, chunkIndex++) {
    if (!deviceConnected) return;  // client left mid-frame
    const size_t count = min(chunkContent, length - offset);
    uint8_t flags = 0;
    if (offset == 0) flags |= VIDEO_FRAME_START;
    if (offset + count == length) flags |= VIDEO_FRAME_END;

    packet[0] = VIDEO_FRAME_MARKER;
    packet[1] = gFrameId;
    packet[2] = chunkIndex & 0xFF;
    packet[3] = (chunkIndex >> 8) & 0xFF;
    packet[4] = flags;
    memcpy(packet + VIDEO_FRAME_HEADER, data + offset, count);
    videoChar->setValue(packet, VIDEO_FRAME_HEADER + count);
    videoChar->notify();
    // Give the controller time to queue this notification before its
    // value is replaced by the next chunk.
    delay(CHUNK_DELAY_MS);
  }
}

// Grab one frame and push it: continuously while the video is on, once
// when a snapshot was requested.
void pumpFrame() {
  if (!deviceConnected || !authed) return;
  bool snapshot = snapshotPending;
  if (!streamOn && !snapshot) return;
  if (!snapshot && millis() - lastFrameMs < FRAME_MIN_INTERVAL_MS) return;

  if (snapshot && !streamOn) {
    // After an idle spell the buffers still hold an old image; flush one
    // grab so the still shows the scene as it is now.
    camera_fb_t* stale = esp_camera_fb_get();
    if (stale) esp_camera_fb_return(stale);
  }
  camera_fb_t* fb = esp_camera_fb_get();
  if (!fb) return;
  // Keep retrying a requested still until a frame is actually captured.
  if (snapshot) snapshotPending = false;
  // pixel_format is PIXFORMAT_JPEG, so fb->buf is a complete JPEG image.
  sendVideoFrame(fb->buf, fb->len);
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
  // BLE never moves more than VGA, so the frame buffers stay small.
  config.frame_size = FRAMESIZE_VGA;
  config.pixel_format = PIXFORMAT_JPEG;
  config.grab_mode = CAMERA_GRAB_WHEN_EMPTY;
  config.fb_location = CAMERA_FB_IN_PSRAM;
  config.jpeg_quality = 12;
  config.fb_count = 1;

  if (psramFound()) {
    config.jpeg_quality = 10;
    config.fb_count = 2;
    config.grab_mode = CAMERA_GRAB_LATEST;
  } else {
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

  sensor_t* s = esp_camera_sensor_get();
  // initial sensors are flipped vertically and colors are a bit saturated
  if (s->id.PID == OV3660_PID) {
    s->set_vflip(s, 1);        // flip it back
    s->set_brightness(s, 1);   // up the brightness just a bit
    s->set_saturation(s, -2);  // lower the saturation
  }
  // Start at QVGA: the sweet spot for BLE throughput.
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

  // With an empty API_KEY the camera starts authorized (no auth write
  // needed); with a key set, every connection must authorize first.
  authed = apiKeyDisabled();
  if (authed) {
    Serial.println("API key check disabled (empty API_KEY)");
  }

  BLEDevice::init(SENSOR_NAME);
  // Video chunks scale with the MTU; ask for the maximum so a capable
  // central can pull ~500-byte chunks instead of 20-byte ones.
  BLEDevice::setMTU(517);
  BLEServer* server = BLEDevice::createServer();
  server->setCallbacks(new ServerCallbacks());

  BLEService* service = server->createService(SERVICE_UUID);

  dataChar = service->createCharacteristic(
    DATA_CHAR_UUID,
    BLECharacteristic::PROPERTY_READ | BLECharacteristic::PROPERTY_NOTIFY);
  dataChar->addDescriptor(new BLE2902());
  dataChar->setCallbacks(new DataCallbacks());

  BLECharacteristic* authChar = service->createCharacteristic(
    AUTH_CHAR_UUID, BLECharacteristic::PROPERTY_WRITE);
  authChar->setCallbacks(new AuthCallbacks());

  // Accept both write flavours, like the display node: a client that can
  // use write-without-response gets a slightly snappier control response.
  BLECharacteristic* cmdChar = service->createCharacteristic(
    CMD_CHAR_UUID,
    BLECharacteristic::PROPERTY_WRITE | BLECharacteristic::PROPERTY_WRITE_NR);
  cmdChar->setCallbacks(new CommandCallbacks());

  videoChar = service->createCharacteristic(
    VIDEO_CHAR_UUID, BLECharacteristic::PROPERTY_NOTIFY);
  videoChar->addDescriptor(new BLE2902());

  service->start();

  BLEAdvertising* advertising = BLEDevice::getAdvertising();
  advertising->addServiceUUID(SERVICE_UUID);
  advertising->setScanResponse(true);
  BLEDevice::startAdvertising();
  Serial.println("BLE advertising started");
  Serial.print("Camera Ready! Advertising as '");
  Serial.print(SENSOR_NAME);
  Serial.println("'");
}

void loop() {
  pumpFrame();
  delay(1);
}
