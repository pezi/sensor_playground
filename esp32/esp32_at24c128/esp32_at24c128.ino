/*
 * Sensor Playground EEPROM Node — ESP32 + AT24C128 (I2C, address 0x50)
 *
 * Implements the *actuator* variant of the Sensor Playground Sensor Interface.
 * The AT24C128 is a 128 Kbit (16 KB) serial EEPROM; this node stores one
 * short text in it. Like the LED the node is the single source of truth:
 * the app asks it to store a text, the node writes the chip in 64-byte
 * pages, reads the region back and reports the *stored* text — which is
 * also sent on connect, so the app opens with what the chip actually
 * holds, surviving power cycles of both ends.
 *
 * EEPROM layout (offset 0):
 *
 *     0-1   magic 'S' 'P'
 *     2-3   text length in bytes (u16 big-endian, max 512)
 *     4-…   UTF-8 text
 *
 * A chip without the magic (e.g. factory-fresh, all 0xFF) reads as an
 * empty text.
 *
 * Over Wi-Fi both directions are JSON messages on the WebSocket:
 *
 *     app -> node   {"write":"Hello"}   store the text
 *                   {"read":true}       re-read the chip and push
 *     node -> app   {"text":"Hello"}    stored text (on connect and after
 *                                       every write/read, read from the chip)
 *
 * Over BLE the stored text is a notify on the data characteristic carrying
 * the same JSON, and a write is staged in offset-addressed binary chunks on
 * the command characteristic (like the SSD1306 bitmap), because a text may
 * not fit in a single ATT write:
 *
 *     0x01 <offset:u16 big-endian> <bytes...>   stage a chunk of UTF-8 text
 *     0x02 <length:u16 big-endian>              store the staged text
 *     0x03                                      re-read the chip and push
 *
 * Chunks must arrive contiguously and the store command carries the total
 * length, so a dropped packet leaves the EEPROM untouched rather than
 * storing a torn text. See ../../python/at24c128/sensor_node.py for the
 * same logic in Python.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app writes the API key to
 *                    AUTH_CHAR_UUID, subscribes to DATA_CHAR_UUID for the
 *                    stored text and writes commands to CMD_CHAR_UUID.
 *
 * Required Libraries:
 *   ArduinoJson, and for TRANSPORT_WIFI: WiFi, WiFiUdp,
 *   WebSockets (by Markus Sattler / Links2004).
 *   (BLE uses the ESP32 core's built-in BLE stack.)
 */

// ============================================================
//  HARDWARE CONFIGURATION
// ============================================================
// I2C address of the AT24C128 (A0..A2 low; 0x50-0x57 depending on strapping).
const uint8_t EEPROM_ADDR = 0x50;

// I2C pins (ESP32 defaults).
const int SDA_PIN = 21;
const int SCL_PIN = 22;

// --- Transport selection (change this line) ---
#define TRANSPORT_WIFI 0
#define TRANSPORT_BLE  1
#ifndef ACTIVE_TRANSPORT
#define ACTIVE_TRANSPORT TRANSPORT_BLE
#endif

#include <ArduinoJson.h>
#include <Wire.h>
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
  #define CMD_CHAR_UUID  "d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f"
#endif

const char* SENSOR_NAME = "AT24C128";

// Command opcodes (see the header comment).
const uint8_t CMD_CHUNK = 0x01;
const uint8_t CMD_WRITE = 0x02;
const uint8_t CMD_READ  = 0x03;

// EEPROM text region: 2-byte magic + 2-byte length + up to 512 bytes UTF-8.
const uint8_t MAGIC_0 = 'S';
const uint8_t MAGIC_1 = 'P';
const size_t HEADER_SIZE = 4;
const size_t TEXT_MAX_BYTES = 512;

// AT24C128 write page; a write transaction must not cross a page boundary.
const size_t PAGE_SIZE = 64;

// Bytes per I2C transaction, within every Wire buffer.
const size_t IO_CHUNK = 32;

// Worst-case internal write cycle per the datasheet is 5 ms; poll for the
// chip's ACK up to this long after each page write.
const unsigned long WRITE_TIMEOUT_MS = 20;

// Text last read from the chip, the single source of truth this node
// publishes.
String gText = "";

void transportPublish();

// Returns the stored-text payload string ({"text":"..."}).
String textJson() {
  StaticJsonDocument<768> doc;
  doc["text"] = gText;
  String out;
  serializeJson(doc, out);
  return out;
}

// ============================================================
//  AT24C128 ACCESS
// ============================================================

// Wait until the chip ACKs its address again — it ignores the bus while an
// internal write cycle is running. Returns false on timeout.
bool eepromWaitReady() {
  unsigned long start = millis();
  while (millis() - start < WRITE_TIMEOUT_MS) {
    Wire.beginTransmission(EEPROM_ADDR);
    if (Wire.endTransmission() == 0) return true;
    delay(1);
  }
  return false;
}

// Random-read [len] bytes starting at [addr] into [out].
bool eepromRead(uint16_t addr, uint8_t* out, size_t len) {
  for (size_t offset = 0; offset < len; offset += IO_CHUNK) {
    size_t n = min(IO_CHUNK, len - offset);
    uint16_t at = addr + offset;
    Wire.beginTransmission(EEPROM_ADDR);
    Wire.write((uint8_t)(at >> 8));
    Wire.write((uint8_t)(at & 0xFF));
    if (Wire.endTransmission(false) != 0) return false;
    if (Wire.requestFrom((int)EEPROM_ADDR, (int)n) != (int)n) return false;
    for (size_t i = 0; i < n; i++) out[offset + i] = Wire.read();
  }
  return true;
}

// Write [len] bytes starting at [addr], splitting the data so no
// transaction crosses a 64-byte page boundary or the Wire buffer.
bool eepromWrite(uint16_t addr, const uint8_t* data, size_t len) {
  size_t offset = 0;
  while (offset < len) {
    uint16_t at = addr + offset;
    size_t page_left = PAGE_SIZE - (at % PAGE_SIZE);
    size_t n = min(min(IO_CHUNK, page_left), len - offset);
    if (!eepromWaitReady()) return false;
    Wire.beginTransmission(EEPROM_ADDR);
    Wire.write((uint8_t)(at >> 8));
    Wire.write((uint8_t)(at & 0xFF));
    Wire.write(data + offset, n);
    if (Wire.endTransmission() != 0) return false;
    offset += n;
  }
  return eepromWaitReady();
}

// Reads the stored text from the chip into gText. A missing magic or an
// implausible length reads as an empty text rather than as garbage.
bool readStoredText() {
  uint8_t header[HEADER_SIZE];
  if (!eepromRead(0, header, HEADER_SIZE)) return false;

  if (header[0] != MAGIC_0 || header[1] != MAGIC_1) {
    gText = "";
    return true;
  }
  size_t len = ((size_t)header[2] << 8) | header[3];
  if (len > TEXT_MAX_BYTES) {
    gText = "";
    return true;
  }

  uint8_t buffer[TEXT_MAX_BYTES + 1];
  if (!eepromRead(HEADER_SIZE, buffer, len)) return false;
  buffer[len] = '\0';
  gText = String((char*)buffer);
  return true;
}

// Stores [len] bytes of UTF-8 text (header + payload) on the chip.
bool storeText(const uint8_t* bytes, size_t len) {
  uint8_t region[HEADER_SIZE + TEXT_MAX_BYTES];
  region[0] = MAGIC_0;
  region[1] = MAGIC_1;
  region[2] = (uint8_t)(len >> 8);
  region[3] = (uint8_t)(len & 0xFF);
  memcpy(region + HEADER_SIZE, bytes, len);
  return eepromWrite(0, region, HEADER_SIZE + len);
}

// ============================================================
//  COMMANDS (transport-independent)
// ============================================================

// Store [bytes] and publish what the chip now actually holds. The publish
// always carries a fresh read-back, so a failed write cannot leave the app
// showing a text the chip never stored.
void applyWrite(const uint8_t* bytes, size_t len) {
  if (len > TEXT_MAX_BYTES) {
    Serial.printf("Rejecting write of %u bytes (max %u)\n",
                  (unsigned)len, (unsigned)TEXT_MAX_BYTES);
  } else if (!storeText(bytes, len)) {
    Serial.println("EEPROM write failed");
  }
  applyRead();
}

// Re-read the chip and publish the stored text.
void applyRead() {
  if (!readStoredText()) {
    Serial.println("EEPROM read failed");
    return;
  }
  transportPublish();
}

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
// ============================================================
//  WIFI TRANSPORT (WebSocket commands + text push, UDP discovery)
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

// Execute one JSON command pushed by the app.
void handleCommand(uint8_t* payload, size_t len) {
  StaticJsonDocument<768> doc;
  // Zero-copy parse; the WebSockets library null-terminates text frames.
  DeserializationError error = deserializeJson(doc, (char*)payload, len);
  if (error) {
    Serial.println("Ignoring malformed command");
    return;
  }

  if (doc["read"] == true) {
    Serial.println("Command: read");
    applyRead();
    return;
  }

  if (!doc["write"].is<const char*>()) {
    Serial.println("Ignoring command without a string 'write'");
    return;
  }
  const char* text = doc["write"];
  Serial.printf("Command: write %u bytes\n", (unsigned)strlen(text));
  applyWrite((const uint8_t*)text, strlen(text));
}

void webSocketEvent(uint8_t num, WStype_t type, uint8_t* payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED: {
      Serial.printf("[%u] Client connected\n", num);
      // Send the stored text so the app opens with the chip's content
      // rather than an empty field. sendTXT takes String& (an lvalue), so
      // hold it in a variable.
      String out = textJson();
      webSocket.sendTXT(num, out);
      break;
    }
    case WStype_DISCONNECTED:
      Serial.printf("[%u] Client disconnected\n", num);
      break;
    case WStype_TEXT:
      handleCommand(payload, len);
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

  // An EEPROM node has no readings to advertise, only its identity.
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

void transportPublish() {
  if (webSocket.connectedClients() == 0) return;
  String out = textJson();
  webSocket.broadcastTXT(out);
  Serial.println(out);
}

#else
// ============================================================
//  BLE TRANSPORT (staged binary commands in, text notify out)
// ============================================================
BLECharacteristic* dataChar = nullptr;
bool deviceConnected = false;
bool authed = false;

// Staging buffer for a chunked write command.
uint8_t gStagedText[TEXT_MAX_BYTES];
// How much of gStagedText is contiguously staged. This is what makes a
// dropped chunk refuse the write instead of storing a torn text.
size_t gStaged = 0;

// Handle one command packet written to CMD_CHAR_UUID.
void handleBleCommand(const uint8_t* packet, size_t len) {
  if (len == 0) {
    Serial.println("Ignoring empty command packet");
    return;
  }

  switch (packet[0]) {
    case CMD_READ:
      Serial.println("Command: read");
      applyRead();
      return;

    case CMD_WRITE: {
      if (len < 3) {
        Serial.println("Ignoring write without a length");
        return;
      }
      size_t total = ((size_t)packet[1] << 8) | packet[2];
      if (total != gStaged) {
        Serial.printf("Refusing write: %u bytes staged, %u announced\n",
                      (unsigned)gStaged, (unsigned)total);
        gStaged = 0;
        return;
      }
      Serial.printf("Command: write %u bytes\n", (unsigned)total);
      applyWrite(gStagedText, total);
      gStaged = 0;
      return;
    }

    case CMD_CHUNK:
      handleChunk(packet, len);
      return;

    default:
      Serial.printf("Ignoring unknown opcode 0x%02X\n", packet[0]);
      return;
  }
}

// Stage one chunk: 0x01 <offset:u16 big-endian> <bytes...>. Chunks must
// arrive contiguously; writing offset 0 restarts the staging.
void handleChunk(const uint8_t* packet, size_t len) {
  if (len < 3) {
    Serial.println("Ignoring chunk without an offset");
    return;
  }
  uint16_t offset = ((uint16_t)packet[1] << 8) | packet[2];
  size_t count = len - 3;
  if ((size_t)offset + count > TEXT_MAX_BYTES) {
    Serial.printf("Ignoring chunk at offset %u overrunning the text "
                  "region (%u bytes)\n",
                  (unsigned)offset, (unsigned)count);
    return;
  }
  if (offset != gStaged) {
    gStaged = 0;
    if (offset != 0) {
      Serial.printf("Ignoring chunk at offset %u, expected %u\n",
                    (unsigned)offset, (unsigned)gStaged);
      return;
    }
  }
  memcpy(gStagedText + offset, packet + 3, count);
  gStaged = offset + count;
}

class ServerCallbacks : public BLEServerCallbacks {
  void onConnect(BLEServer* server) override { deviceConnected = true; }
  void onDisconnect(BLEServer* server) override {
    deviceConnected = false;
    authed = false;
    gStaged = 0;
    server->getAdvertising()->start();  // allow the next client to find us
  }
};

// Client must write the shared API key here before the chip can be used.
class AuthCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    String val = characteristic->getValue();
    while (val.length() > 0 && (val[val.length() - 1] == '\0' || val[val.length() - 1] == '\r' || val[val.length() - 1] == '\n')) {
      val.remove(val.length() - 1);
    }
    authed = (val == API_KEY);
    Serial.println(authed ? "Client authorized" : "Bad API key");
    // Push the stored text right after a successful authorization.
    if (authed) {
      notifySensorJson(dataChar, textJson());
    }
  }
};

// Serve the stored text only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? textJson().c_str() : "{}");
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

  // Accept both write flavours, like the display node: the app uses
  // acknowledged writes so the staged chunks arrive in order.
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

void transportPublish() {
  if (!deviceConnected || !authed) return;
  String out = textJson();
  notifySensorJson(dataChar, out);
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Playground AT24C128 EEPROM Node ---");

  Wire.begin(SDA_PIN, SCL_PIN);
  Wire.setClock(400000);

  if (!readStoredText()) {
    Serial.println("AT24C128 not responding — check wiring and address");
  } else {
    Serial.printf("Stored text: %u bytes\n", (unsigned)gText.length());
  }

  transportSetup();

  Serial.print("Actuator: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();
  delay(1);
}
