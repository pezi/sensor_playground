/*
 * Sensor Playground Sensor Node — ESP32 + Grove NFC Tag (M24LR64E-R)
 *
 * Implements a *push* variant of the Sensor Playground Sensor Interface. The
 * Grove NFC Tag is a passive dual-interface EEPROM: a phone or NFC writer
 * stores an NDEF message over the ISO 15693 RF interface, and this node
 * reads the same memory over I2C (user memory at address 0x53). It polls
 * the NDEF area once a second and pushes one JSON message whenever the
 * content changes:
 *
 *     {"kind":"text","value":"Hello"}
 *     {"kind":"uri","value":"https://seeed.cc"}
 *     {"kind":"data","value":"DEADBEEF"}       (hex, truncated)
 *     {"kind":"empty"}
 *
 * Unlike the pure event sensors the tag holds state, so the current
 * content is also sent to every client right after it connects (Wi-Fi) or
 * authorizes (BLE), and served on BLE reads of the data characteristic.
 * See ../../python/nfctag/sensor_node.py for the same logic in Python.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then subscribes to
 *                    DATA_CHAR_UUID; each change arrives as one notify.
 *
 * Required Libraries:
 *   ArduinoJson, Wire,
 *   and for TRANSPORT_WIFI: WiFi, WiFiUdp,
 *   WebSockets (by Markus Sattler / Links2004).
 *   (BLE uses the ESP32 core's built-in BLE stack.)
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
#else
  #include <BLEDevice.h>
  #include <BLEServer.h>
  #include <BLEUtils.h>
  #include <BLE2902.h>
  #include "../common/sensor_ble_framing.h"

  // Shared Sensor Playground GATT contract (must match the app's BleUuids).
  #define SERVICE_UUID   "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
  #define DATA_CHAR_UUID "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
  #define AUTH_CHAR_UUID "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
#endif

const char* SENSOR_NAME = "NFCTAG";

// M24LR64E-R user memory (system area answers at 0x57 and is not used).
const uint8_t TAG_I2C_ADDRESS = 0x53;

// Bytes of the EEPROM scanned for the NDEF message (CC + TLV area), read
// in chunks that fit the Wire buffer.
const size_t SCAN_LENGTH = 256;
const size_t SCAN_CHUNK = 32;

// Milliseconds between EEPROM polls; an RF write shows on the next poll.
const unsigned long POLL_INTERVAL_MS = 1000;

// Cap for hex dumps of unparseable payloads (bytes before hex encoding).
const size_t DATA_HEX_CAP = 64;

// The JSON for the tag's current content; pushed on change and served to
// newly connected clients. Empty until the first successful scan.
String gContentJson;

void transportPublish(const String& json);

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
// ============================================================
//  WIFI TRANSPORT (WebSocket push + UDP discovery)
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

void webSocketEvent(uint8_t num, WStype_t type, uint8_t* payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED:
      Serial.printf("[%u] Client connected\n", num);
      // The tag holds state: a client that just connected gets the
      // current content instead of waiting for the next RF write.
      if (gContentJson.length() > 0) {
        webSocket.sendTXT(num, gContentJson);
      }
      break;
    case WStype_DISCONNECTED:
      Serial.printf("[%u] Client disconnected\n", num);
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

  // An NFC tag node has no live readings to advertise, only its identity.
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

void transportPublish(const String& json) {
  Serial.println(json);
  if (webSocket.connectedClients() == 0) return;
  // sendTXT/broadcastTXT take a non-const String&, so pass a copy.
  String out = json;
  webSocket.broadcastTXT(out);
}

#else
// ============================================================
//  BLE TRANSPORT (notify per change)
// ============================================================
BLECharacteristic* dataChar = nullptr;
bool deviceConnected = false;
bool authed = false;

class ServerCallbacks : public BLEServerCallbacks {
  void onConnect(BLEServer* server) override { deviceConnected = true; }
  void onDisconnect(BLEServer* server) override {
    deviceConnected = false;
    authed = false;
    server->getAdvertising()->start();  // allow the next client to find us
  }
};

// Client must write the shared API key here before content is served.
class AuthCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    String val = characteristic->getValue();
    while (val.length() > 0 && (val[val.length() - 1] == '\0' || val[val.length() - 1] == '\r' || val[val.length() - 1] == '\n')) {
      val.remove(val.length() - 1);
    }
    authed = (val == API_KEY);
    Serial.println(authed ? "Client authorized" : "Bad API key");
    // The tag holds state: push the current content right after a
    // successful authorization, like the LED node pushes its state.
    if (authed && gContentJson.length() > 0) {
      notifySensorJson(dataChar, gContentJson);
    }
  }
};

// Serve the current content only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(
      (authed && gContentJson.length() > 0) ? gContentJson.c_str() : "{}");
  }
};

void transportSetup() {
  BLEDevice::init(SENSOR_NAME);
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

void transportPublish(const String& json) {
  Serial.println(json);
  if (!deviceConnected || !authed) return;
  notifySensorJson(dataChar, json);
}
#endif

// ============================================================
//  M24LR64E access + NDEF parsing
// ============================================================

// Reads [length] bytes of user memory starting at [address] (16-bit
// addressing, chunked to fit the Wire buffer). Returns false on any I2C
// error, leaving the buffer content undefined.
bool readTagMemory(uint16_t address, uint8_t* buffer, size_t length) {
  for (size_t offset = 0; offset < length; offset += SCAN_CHUNK) {
    const uint16_t chunkAddress = address + offset;
    Wire.beginTransmission(TAG_I2C_ADDRESS);
    Wire.write((uint8_t)(chunkAddress >> 8));
    Wire.write((uint8_t)(chunkAddress & 0xFF));
    if (Wire.endTransmission(false) != 0) return false;

    const size_t count = min(SCAN_CHUNK, length - offset);
    if (Wire.requestFrom((int)TAG_I2C_ADDRESS, (int)count) != (int)count) {
      return false;
    }
    for (size_t i = 0; i < count; i++) {
      buffer[offset + i] = Wire.read();
    }
  }
  return true;
}

// NFC Forum URI record prefix codes (the common subset; the same table
// lives in the Python node — keep them identical).
const char* uriPrefix(uint8_t code) {
  switch (code) {
    case 0x01: return "http://www.";
    case 0x02: return "https://www.";
    case 0x03: return "http://";
    case 0x04: return "https://";
    case 0x05: return "tel:";
    case 0x06: return "mailto:";
    default:   return "";
  }
}

String contentJson(const char* kind, const String* value) {
  StaticJsonDocument<512> doc;
  doc["kind"] = kind;
  if (value != nullptr) doc["value"] = *value;
  String out;
  serializeJson(doc, out);
  return out;
}

// Hex-dump fallback for content that is not a text or URI record.
String dataJson(const uint8_t* data, size_t length) {
  String hex;
  const size_t capped = min(length, DATA_HEX_CAP);
  for (size_t i = 0; i < capped; i++) {
    char pair[3];
    snprintf(pair, sizeof(pair), "%02X", data[i]);
    hex += pair;
  }
  return contentJson("data", &hex);
}

// Parses the first record of an NDEF message into the content JSON.
String parseNdefRecord(const uint8_t* message, size_t length) {
  if (length < 3) return dataJson(message, length);
  const uint8_t flags = message[0];
  const uint8_t tnf = flags & 0x07;
  const bool shortRecord = flags & 0x10;
  const bool hasId = flags & 0x08;
  const size_t typeLength = message[1];
  size_t offset = 2;

  uint32_t payloadLength;
  if (shortRecord) {
    payloadLength = message[offset];
    offset += 1;
  } else {
    if (offset + 4 > length) return dataJson(message, length);
    payloadLength = ((uint32_t)message[offset] << 24) |
                    ((uint32_t)message[offset + 1] << 16) |
                    ((uint32_t)message[offset + 2] << 8) |
                    message[offset + 3];
    offset += 4;
  }
  size_t idLength = 0;
  if (hasId) {
    if (offset >= length) return dataJson(message, length);
    idLength = message[offset];
    offset += 1;
  }
  if (offset + typeLength + idLength > length) {
    return dataJson(message, length);
  }
  const uint8_t* type = message + offset;
  offset += typeLength + idLength;
  if (offset + payloadLength > length) {
    return dataJson(message + offset, length - offset);
  }
  const uint8_t* payload = message + offset;

  if (tnf == 0x01 && typeLength == 1 && type[0] == 'T' && payloadLength > 0) {
    const uint8_t status = payload[0];
    if (status & 0x80) {
      // UTF-16 text is rare (phones write UTF-8); report it raw rather
      // than decoding multi-byte characters on the microcontroller.
      return dataJson(payload, payloadLength);
    }
    const size_t langLength = status & 0x3F;
    if (1 + langLength > payloadLength) return dataJson(payload, payloadLength);
    String text;
    for (size_t i = 1 + langLength; i < payloadLength; i++) {
      text += (char)payload[i];
    }
    return contentJson("text", &text);
  }
  if (tnf == 0x01 && typeLength == 1 && type[0] == 'U' && payloadLength > 0) {
    String uri = uriPrefix(payload[0]);
    for (size_t i = 1; i < payloadLength; i++) {
      uri += (char)payload[i];
    }
    return contentJson("uri", &uri);
  }
  return dataJson(payload, payloadLength);
}

// Parses the scanned EEPROM area (Type 5 capability container + TLV
// stream) into the content JSON. Anything that fails to parse is
// reported as a hex dump rather than dropped, so the app always sees
// that *something* was written.
String parseNdefArea(const uint8_t* data, size_t length) {
  if (length < 4 || (data[0] != 0xE1 && data[0] != 0xE2)) {
    for (size_t i = 0; i < length; i++) {
      if (data[i] != 0x00 && data[i] != 0xFF) return dataJson(data, length);
    }
    return contentJson("empty", nullptr);
  }

  size_t offset = 4;  // first byte after the capability container
  while (offset < length) {
    const uint8_t tlv = data[offset];
    if (tlv == 0x00) {  // padding
      offset += 1;
      continue;
    }
    if (tlv == 0xFE) {  // terminator: no NDEF TLV found
      return contentJson("empty", nullptr);
    }
    if (tlv != 0x03) {  // unknown TLV: skip it (1-byte length format)
      if (offset + 1 >= length) return contentJson("empty", nullptr);
      offset += 2 + data[offset + 1];
      continue;
    }
    // NDEF message TLV: 1-byte length, or 0xFF + 2-byte big-endian.
    if (offset + 1 >= length) return contentJson("empty", nullptr);
    uint32_t messageLength = data[offset + 1];
    offset += 2;
    if (messageLength == 0xFF) {
      if (offset + 2 > length) return contentJson("empty", nullptr);
      messageLength = ((uint32_t)data[offset] << 8) | data[offset + 1];
      offset += 2;
    }
    if (messageLength == 0) return contentJson("empty", nullptr);
    if (offset + messageLength > length) {
      // Truncated by the scan window.
      return dataJson(data + offset, length - offset);
    }
    return parseNdefRecord(data + offset, messageLength);
  }
  return contentJson("empty", nullptr);
}

// Scans the tag and publishes the content when it changed.
void pollTag() {
  static uint8_t buffer[SCAN_LENGTH];
  static unsigned long lastPollMs = 0;

  if (millis() - lastPollMs < POLL_INTERVAL_MS && lastPollMs != 0) return;
  lastPollMs = millis();

  if (!readTagMemory(0, buffer, SCAN_LENGTH)) {
    Serial.println("Tag read failed");
    return;
  }
  String json = parseNdefArea(buffer, SCAN_LENGTH);
  if (json != gContentJson) {
    gContentJson = json;
    transportPublish(gContentJson);
  }
}

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Playground Sensor Node ---");

  Wire.begin();
  Wire.beginTransmission(TAG_I2C_ADDRESS);
  if (Wire.endTransmission() != 0) {
    Serial.println("Error: M24LR64E not found!");
    while (1) delay(1000);
  }

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();
  pollTag();
  delay(1);
}
