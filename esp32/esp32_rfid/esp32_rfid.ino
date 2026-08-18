/*
 * Sensor Tester Sensor Node — ESP32 + Grove 125KHz RFID Reader
 *
 * Implements a *push* variant of the Sensor Tester Sensor Interface. Unlike
 * the environment sensors (which serve readings on request), an RFID reader
 * only produces data at the instant a tag is scanned, so this node pushes
 * one JSON message per scanned EM4100 tag:
 *
 *     {"tag":"0F0024ADAB"}
 *
 * The value is the 10 ASCII-hex data characters of the RDM630-style frame
 * the reader emits over its 9600-baud UART (the reader's jumper must be on
 * UART mode, not Wiegand):
 *
 *     STX 0x02 | 10 hex data chars | 2 hex checksum chars | ETX 0x03
 *
 * The checksum byte is the XOR of the five data bytes; the node verifies it
 * and drops corrupt frames. The reader repeats the frame while a tag is
 * held near the antenna, so repeats of the same tag are suppressed for
 * REPEAT_SUPPRESS_MS. See ../../python/rfid/sensor_node.py for the same
 * logic in Python.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then subscribes to
 *                    DATA_CHAR_UUID; each scan arrives as one notify.
 *
 * Required Libraries:
 *   ArduinoJson,
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

  // Shared Sensor Tester GATT contract (must match the app's BleUuids).
  #define SERVICE_UUID   "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
  #define DATA_CHAR_UUID "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
  #define AUTH_CHAR_UUID "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
#endif

const char* SENSOR_NAME = "RFID";

// UART wiring: the reader's TX goes to the ESP32's RX2 (GPIO 16). The
// reader is transmit-only, so the TX pin of Serial2 stays unused.
const int RFID_RX_PIN = 16;
const int RFID_TX_PIN = 17;

// RDM630 frame markers and length (10 data + 2 checksum hex chars).
const uint8_t FRAME_STX = 0x02;
const uint8_t FRAME_ETX = 0x03;
const size_t FRAME_HEX_CHARS = 12;

// Suppress repeats of the same tag while it is held near the antenna.
const unsigned long REPEAT_SUPPRESS_MS = 2000;

const char* readTagFrame();
void transportPublish(const char* tag);

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

  // An RFID node has no live readings to advertise, only its identity.
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

void transportPublish(const char* tag) {
  if (webSocket.connectedClients() == 0) return;
  StaticJsonDocument<64> doc;
  doc["tag"] = tag;
  String out;
  serializeJson(doc, out);
  webSocket.broadcastTXT(out);
  Serial.println(out);
}

#else
// ============================================================
//  BLE TRANSPORT (notify per scan)
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

// Client must write the shared API key here before scans are served.
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

// Never serve the cached last scan to an unauthorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    if (!authed) characteristic->setValue("{}");
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

void transportPublish(const char* tag) {
  if (!deviceConnected || !authed) return;
  StaticJsonDocument<64> doc;
  doc["tag"] = tag;
  String out;
  serializeJson(doc, out);
  notifySensorJson(dataChar, out);
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  // The reader is silent until a tag appears, so unlike the I2C sensors
  // there is nothing to probe here; a wiring error only shows as silence.
  Serial2.begin(9600, SERIAL_8N1, RFID_RX_PIN, RFID_TX_PIN);

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();

  const char* tag = readTagFrame();
  if (tag != nullptr) {
    static String gLastTag;
    static unsigned long gLastPublishMs = 0;
    if (gLastTag != tag || millis() - gLastPublishMs >= REPEAT_SUPPRESS_MS) {
      transportPublish(tag);
      gLastTag = tag;
      gLastPublishMs = millis();
    }
  }
  delay(1);
}

// ============================================================
// Feed the UART into the RDM630 frame state machine. Returns the 10-char
// uppercased tag once a checksum-valid frame completes, else nullptr. The
// machine resyncs on any STX and drops the frame on non-hex noise or
// overflow, so garbage on the line can never block the loop.
// ============================================================

// XOR of the five data bytes must equal the checksum byte.
bool frameChecksumOk(const char* hex12) {
  uint8_t checksum = 0;
  for (size_t i = 0; i < FRAME_HEX_CHARS; i += 2) {
    char pair[3] = {hex12[i], hex12[i + 1], '\0'};
    uint8_t value = (uint8_t)strtoul(pair, nullptr, 16);
    if (i < 10) {
      checksum ^= value;
    } else {
      return checksum == value;
    }
  }
  return false;
}

const char* readTagFrame() {
  static char frame[FRAME_HEX_CHARS + 1];
  static int framePos = -1;  // -1 = waiting for STX
  static char tag[11];

  while (Serial2.available() > 0) {
    uint8_t byte = Serial2.read();

    if (byte == FRAME_STX) {
      framePos = 0;  // resync, also on a second STX mid-frame
      continue;
    }
    if (framePos < 0) continue;  // noise outside a frame

    if (byte == FRAME_ETX) {
      bool complete = framePos == (int)FRAME_HEX_CHARS;
      framePos = -1;
      if (!complete) continue;
      frame[FRAME_HEX_CHARS] = '\0';
      if (!frameChecksumOk(frame)) {
        Serial.println("Dropping RFID frame with bad checksum");
        continue;
      }
      memcpy(tag, frame, 10);
      tag[10] = '\0';
      return tag;
    }

    if (!isxdigit(byte) || framePos >= (int)FRAME_HEX_CHARS) {
      framePos = -1;  // non-hex noise mid-frame, or overflow
      continue;
    }
    frame[framePos++] = toupper(byte);
  }
  return nullptr;
}
