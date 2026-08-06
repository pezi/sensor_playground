/*
 * Sensor Tester Display Node — ESP32 + SSD1306 (128x64 I2C OLED)
 *
 * Implements the *display* variant of the Sensor Tester Sensor Interface.
 * Unlike sensor nodes this node consumes data: the app pushes one command
 * per action and the node draws it on the panel. Over Wi-Fi a command is
 * one JSON message:
 *
 *     {"id":1,"image":"<base64>"}   show a bitmap (1024 bytes, see below)
 *     {"id":2,"clear":true}         blank the display
 *
 * After applying a command the node replies with
 * `{"id":<same id>,"ok":true}` or an error ACK with `ok:false`.
 *
 * Bitmap format (matches the dart_periphery SSD1306 example):
 * 128x64 pixels, 1 bit per pixel, packed horizontally row by row —
 * 16 bytes per row, MSB of each byte is the leftmost pixel (the
 * image2cpp "horizontal" byte orientation). The sketch transposes this
 * into the SSD1306 native page format before writing it over I2C.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then writes commands to
 *                    CMD_CHAR_UUID.
 *
 * Over BLE a frame cannot travel as one message: an ATT write carries at
 * most MTU-3 bytes, so the app sends the bitmap as a series of raw binary
 * chunks (no base64 — BLE writes are binary-safe, which keeps the transfer
 * at 1024 bytes instead of ~1.4 KB) followed by a show command:
 *
 *     0x01 <offset:u16 big-endian> <bytes...>   stage a chunk
 *     0x02                                      draw the staged frame
 *     0x03                                      blank the display
 *
 * Chunks must arrive contiguously; writing offset 0 restarts the frame, so
 * a client that gives up mid-frame simply starts over. See FrameAssembler
 * in ../../python/ssd1306/sensor_node.py for the same logic in Python.
 *
 * Required Libraries:
 *   ArduinoJson, Wire,
 *   and for TRANSPORT_WIFI: WiFi, WiFiUdp,
 *   WebSockets (by Markus Sattler / Links2004).
 *   (BLE uses the ESP32 core's built-in BLE stack.)
 *   The SSD1306 is driven with raw I2C writes — no display library needed.
 */

// --- Transport selection (change this line) ---
#define TRANSPORT_WIFI 0
#define TRANSPORT_BLE  1
#ifndef ACTIVE_TRANSPORT
#define ACTIVE_TRANSPORT TRANSPORT_BLE
#endif

#include <Wire.h>
#include <ArduinoJson.h>
#include "secrets.h"

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
  #include <WiFi.h>
  #include "../common/sensor_wifi_runtime.h"
  #include <WiFiUdp.h>
  #include <WebSocketsServer.h>
  #include "mbedtls/base64.h"
#else
  #include <BLEDevice.h>
  #include <BLEServer.h>
  #include <BLEUtils.h>
  #include <BLE2902.h>
  #include "../common/sensor_ble_framing.h"

  // Shared Sensor Tester GATT contract (must match the app's BleUuids).
  #define SERVICE_UUID   "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
  #define DATA_CHAR_UUID "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
  #define AUTH_CHAR_UUID "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
  #define CMD_CHAR_UUID  "d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f"
#endif

const char* SENSOR_NAME = "SSD1306";

// ============================================================
//  SSD1306 driver (raw I2C, port of dart_periphery's ssd1306.dart)
// ============================================================
const uint8_t OLED_ADDRESS = 0x3C;  // some boards are strapped to 0x3D

const int OLED_WIDTH  = 128;
const int OLED_HEIGHT = 64;
const int FRAME_SIZE  = OLED_WIDTH * OLED_HEIGHT / 8;  // 1024
const int ROW_BYTES   = OLED_WIDTH / 8;                // 16

const uint8_t INIT_SEQUENCE[] = {
  0xAE, 0xD5, 0x80, 0xA8, 0x3F, 0xD3, 0x00, 0x40, 0x8D, 0x14, 0x20, 0x00,
  0xA1, 0xC8, 0xDA, 0x12, 0x81, 0xCF, 0xD9, 0xF1, 0xDB, 0x40, 0xA4, 0xA6,
  0xAF
};

// Decoded bitmap (horizontal format) and its page-format transposition.
static uint8_t gBitmap[FRAME_SIZE];
static uint8_t gNative[FRAME_SIZE];

// Write bytes prefixed with a control byte (0x00 commands, 0x40 data),
// chunked to respect the Wire transmit buffer.
bool oledWrite(uint8_t control, const uint8_t* data, size_t len) {
  const size_t CHUNK = 16;
  for (size_t offset = 0; offset < len; offset += CHUNK) {
    size_t n = min(CHUNK, len - offset);
    Wire.beginTransmission(OLED_ADDRESS);
    Wire.write(control);
    Wire.write(data + offset, n);
    if (Wire.endTransmission() != 0) return false;
  }
  return true;
}

// Reset the internal memory write index to the begin.
bool oledResetPosition() {
  const uint8_t reset[] = {0xB0, 0x00, 0x10};
  return oledWrite(0x00, reset, sizeof(reset));
}

bool oledClear() {
  if (!oledResetPosition()) return false;
  memset(gNative, 0x00, FRAME_SIZE);
  return oledWrite(0x40, gNative, FRAME_SIZE);
}

bool oledInit() {
  return oledWrite(0x00, INIT_SEQUENCE, sizeof(INIT_SEQUENCE)) && oledClear();
}

// Transpose one column byte: bit `j` of 8 stacked rows starting at `index`.
uint8_t convertByte(int index, int j) {
  uint8_t mask = 1 << j;
  uint8_t byte = 0;
  for (int i = 0; i < 8; i++) {
    if (gBitmap[index + ROW_BYTES * i] & mask) {
      byte |= (1 << i);
    }
  }
  return byte;
}

// Transpose the horizontal MSB-first bitmap in gBitmap into the SSD1306
// native page format (8 pages x 128 column bytes, LSB on top) and show it.
bool oledShowBitmap() {
  int index = 0;
  int count = 0;
  for (int y = 0; y < OLED_HEIGHT / 8; ++y) {
    int pos = index;
    for (int i = 0; i < ROW_BYTES; ++i) {
      for (int j = 7; j >= 0; --j) {
        gNative[count++] = convertByte(pos, j);
      }
      ++pos;
    }
    index += OLED_WIDTH;
  }
  if (!oledResetPosition()) return false;
  return oledWrite(0x40, gNative, FRAME_SIZE);
}

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
// ============================================================
//  WIFI TRANSPORT (WebSocket commands + UDP discovery)
// ============================================================
const int WS_PORT  = 9132;
const int UDP_PORT = 9133;

WiFiUDP udp;
WebSocketsServer webSocket = WebSocketsServer(WS_PORT);

// Clients must present this header on the WebSocket handshake.
const char* MANDATORY_HEADERS[] = {"X-Api-Key"};
const size_t MANDATORY_HEADER_COUNT = 1;

bool validateApiKey(String headerName, String headerValue) {
  if (headerName.equalsIgnoreCase("X-Api-Key")) {
    headerValue.trim();
    return headerValue == String(API_KEY);
  }
  return true;
}

void sendCommandAck(uint8_t client, int commandId, bool ok,
                    const char* error = nullptr) {
  StaticJsonDocument<192> ack;
  ack["id"] = commandId;
  ack["ok"] = ok;
  if (!ok && error != nullptr) ack["error"] = error;
  String response;
  serializeJson(ack, response);
  webSocket.sendTXT(client, response);
}

// Execute one JSON command pushed by the app and acknowledge its outcome.
void handleCommand(uint8_t client, uint8_t* payload, size_t len) {
  StaticJsonDocument<512> doc;
  // Zero-copy parse; the WebSockets library null-terminates text frames.
  DeserializationError error = deserializeJson(doc, (char*)payload, len);
  if (error) {
    Serial.println("Ignoring malformed command");
    sendCommandAck(client, -1, false, "malformed JSON");
    return;
  }

  int commandId = doc["id"] | -1;
  if (commandId < 0) {
    sendCommandAck(client, -1, false, "missing command id");
    return;
  }

  if (doc["clear"] == true) {
    Serial.println("Command: clear");
    const bool applied = oledClear();
    sendCommandAck(
        client, commandId, applied,
        applied ? nullptr : "display I2C write failed");
    return;
  }

  const char* image = doc["image"];
  if (image == nullptr) {
    sendCommandAck(client, commandId, false, "unknown command");
    return;
  }

  size_t decoded = 0;
  int result = mbedtls_base64_decode(gBitmap, sizeof(gBitmap), &decoded,
                                     (const unsigned char*)image,
                                     strlen(image));
  if (result != 0 || decoded != FRAME_SIZE) {
    Serial.printf("Ignoring image (decode result %d, %u bytes)\n",
                  result, (unsigned)decoded);
    sendCommandAck(client, commandId, false, "invalid image");
    return;
  }
  Serial.println("Command: image");
  const bool applied = oledShowBitmap();
  sendCommandAck(
      client, commandId, applied,
      applied ? nullptr : "display I2C write failed");
}

void webSocketEvent(uint8_t num, WStype_t type, uint8_t* payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED:
      Serial.printf("[%u] Client connected\n", num);
      break;
    case WStype_DISCONNECTED:
      Serial.printf("[%u] Client disconnected\n", num);
      break;
    case WStype_TEXT:
      handleCommand(num, payload, len);
      break;
    default:
      break;
  }
}

void transportSetup() {
  if (!connectSensorWifi(WIFI_SSID, WIFI_PASS)) {
    Serial.println("Restarting after WiFi setup failure");
    delay(1000);
    ESP.restart();
  }

  webSocket.begin();
  webSocket.onEvent(webSocketEvent);
  webSocket.onValidateHttpHeader(validateApiKey, MANDATORY_HEADERS,
                                 MANDATORY_HEADER_COUNT);

  udp.begin(UDP_PORT);
  Serial.println("WebSocket on port 9132, UDP on port 9133");
}

void handleUdpDiscovery() {
  int packetSize = udp.parsePacket();
  if (!packetSize) return;

  char buffer[64];
  int len = udp.read(buffer, sizeof(buffer) - 1);
  buffer[len] = '\0';
  if (strstr(buffer, "SENSOR_TESTER") == NULL) return;

  // A display node has no readings to advertise, only its identity.
  StaticJsonDocument<256> doc;
  doc["type"] = SENSOR_NAME;
  doc["host"] = HOSTNAME;
  doc["ip"]   = WiFi.localIP().toString();
  doc["port"] = WS_PORT;
  String json;
  serializeJson(doc, json);

  udp.beginPacket(udp.remoteIP(), udp.remotePort());
  udp.print(json);
  udp.endPacket();
}

void transportLoop() {
  if (!sensorWifiReady()) {
    delay(10);
    return;
  }
  webSocket.loop();
  handleUdpDiscovery();
}

#else
// ============================================================
//  BLE TRANSPORT (chunked binary commands)
// ============================================================
bool deviceConnected = false;
bool authed = false;

// Command opcodes (see the header comment).
const uint8_t CMD_CHUNK = 0x01;
const uint8_t CMD_SHOW  = 0x02;
const uint8_t CMD_CLEAR = 0x03;

// How much of gBitmap is contiguously staged. This is what makes a partial
// frame detectable: without it a dropped chunk would be drawn as a band of
// stale pixels left over from the previous image.
static uint16_t gStaged = 0;

// Handle one command packet written to CMD_CHAR_UUID.
void handleBleCommand(const uint8_t* packet, size_t len) {
  if (len == 0) {
    Serial.println("Ignoring empty command packet");
    return;
  }

  switch (packet[0]) {
    case CMD_CLEAR:
      gStaged = 0;
      Serial.println("Command: clear");
      if (!oledClear()) Serial.println("Display I2C write failed");
      return;

    case CMD_SHOW:
      if (gStaged != FRAME_SIZE) {
        Serial.printf("Ignoring show with an incomplete frame (%u of %d)\n",
                      (unsigned)gStaged, FRAME_SIZE);
        gStaged = 0;
        return;
      }
      gStaged = 0;
      Serial.println("Command: image");
      if (!oledShowBitmap()) Serial.println("Display I2C write failed");
      return;

    case CMD_CHUNK:
      break;  // handled below

    default:
      Serial.printf("Ignoring unknown opcode 0x%02X\n", packet[0]);
      return;
  }

  if (len < 3) {
    Serial.println("Ignoring chunk without an offset");
    return;
  }
  uint16_t offset = ((uint16_t)packet[1] << 8) | packet[2];
  size_t count = len - 3;
  if ((size_t)offset + count > FRAME_SIZE) {
    Serial.printf("Ignoring chunk at offset %u overrunning the frame (%u bytes)\n",
                  (unsigned)offset, (unsigned)count);
    return;
  }
  if (offset != gStaged) {
    // Offset 0 is a deliberate restart, anything else is a gap.
    if (offset != 0) {
      Serial.printf("Ignoring chunk at offset %u, expected %u\n",
                    (unsigned)offset, (unsigned)gStaged);
      gStaged = 0;
      return;
    }
  }
  memcpy(gBitmap + offset, packet + 3, count);
  gStaged = offset + count;
}

class ServerCallbacks : public BLEServerCallbacks {
  void onConnect(BLEServer* server) override { deviceConnected = true; }
  void onDisconnect(BLEServer* server) override {
    deviceConnected = false;
    authed = false;
    gStaged = 0;  // a half-sent frame must not survive into the next session
    server->getAdvertising()->start();  // allow the next client to find us
  }
};

// Client must write the shared API key here before commands are accepted.
class AuthCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    String val = characteristic->getValue();
    while (val.length() > 0 && (val[val.length() - 1] == '\0' || val[val.length() - 1] == '\r' || val[val.length() - 1] == '\n')) {
      val.remove(val.length() - 1);
    }
    authed = (val == API_KEY);
    Serial.println(authed ? "Client authorized" : "Bad API key");
  }
};

class CommandCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    if (!authed) return;  // commands are gated like the sensor nodes' reads
    handleBleCommand(characteristic->getData(), characteristic->getLength());
  }
};

void transportSetup() {
  BLEDevice::init(SENSOR_NAME);
  // A frame is 1024 bytes; at the default 23-byte MTU it would take ~52
  // writes. Ask for the maximum so the app can send ~500 at a time.
  BLEDevice::setMTU(517);

  BLEServer* server = BLEDevice::createServer();
  server->setCallbacks(new ServerCallbacks());

  BLEService* service = server->createService(SERVICE_UUID);

  // A display node produces no readings, but it carries the data
  // characteristic anyway so every Sensor Tester node exposes the same
  // three characteristics and the app can connect with one code path.
  BLECharacteristic* dataChar = service->createCharacteristic(
    DATA_CHAR_UUID,
    BLECharacteristic::PROPERTY_READ | BLECharacteristic::PROPERTY_NOTIFY);
  dataChar->addDescriptor(new BLE2902());
  dataChar->setValue("{}");

  BLECharacteristic* authChar = service->createCharacteristic(
    AUTH_CHAR_UUID, BLECharacteristic::PROPERTY_WRITE);
  authChar->setCallbacks(new AuthCallbacks());

  // Accept both write flavours: a frame is many writes, and clients that can
  // use write-without-response send it far faster.
  BLECharacteristic* cmdChar = service->createCharacteristic(
    CMD_CHAR_UUID,
    BLECharacteristic::PROPERTY_WRITE | BLECharacteristic::PROPERTY_WRITE_NR);
  cmdChar->setCallbacks(new CommandCallbacks());

  service->start();

  BLEAdvertising* advertising = BLEDevice::getAdvertising();
  advertising->addServiceUUID(SERVICE_UUID);
  advertising->setScanResponse(true);
  BLEDevice::startAdvertising();
  Serial.println("BLE advertising started");
}

void transportLoop() {
  delay(1);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Display Node ---");

  Wire.begin();
  Wire.beginTransmission(OLED_ADDRESS);
  if (Wire.endTransmission() != 0) {
    Serial.println("Error: SSD1306 not found!");
    while (1) delay(1000);
  }
  if (!oledInit()) {
    Serial.println("Error: SSD1306 initialization failed!");
    while (1) delay(1000);
  }

  transportSetup();

  Serial.print("Display: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();
  delay(1);
}
