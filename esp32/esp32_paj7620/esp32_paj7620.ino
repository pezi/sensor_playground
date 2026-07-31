/*
 * Sensor Tester Sensor Node — ESP32 + PAJ7620U2 (Grove Gesture)
 *
 * Implements a *push* variant of the Sensor Tester Sensor Interface. Unlike the
 * environment sensors (which serve readings on request), a gesture sensor only
 * produces data at the instant a gesture happens, so this node pushes one
 * JSON message per detected gesture:
 *
 *     {"gesture":"forward"}
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then subscribes to
 *                    DATA_CHAR_UUID; each gesture arrives as one notify.
 *
 * The gesture strings match the app's Gesture enum names exactly:
 *   forward, forwardBackward, backward, backwardForward,
 *   right, rightLeft, left, leftRight,
 *   up, upDown, down, downUp,
 *   clockwise, antiClockwise, wave
 *
 * Required Libraries:
 *   ArduinoJson, Wire, Gesture PAJ7620 (Seeed Studio),
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
#include "paj7620.h"
#include "secrets.h"

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
  #include <WiFi.h>
  #include <WiFiUdp.h>
  #include <WebSocketsServer.h>
#else
  #include <BLEDevice.h>
  #include <BLEServer.h>
  #include <BLEUtils.h>
  #include <BLE2902.h>

  // Shared Sensor Tester GATT contract (must match the app's BleUuids).
  #define SERVICE_UUID   "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
  #define DATA_CHAR_UUID "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
  #define AUTH_CHAR_UUID "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
#endif

const char* SENSOR_NAME = "PAJ7620";

// Window to wait for a reverse swipe before reporting a combined gesture.
// (The Seeed PAJ7620 library defines the GES_*_FLAG constants but leaves this
// timing to the sketch, as in their reference example.)
#define GES_ENTRY_TIME 800

const char* readGesture();
void transportPublish(const char* gesture);

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
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  Serial.print("Connecting to WiFi");
  while (WiFi.status() != WL_CONNECTED) {
    delay(500);
    Serial.print(".");
  }
  Serial.println("\nWiFi connected.");
  Serial.print("IP: ");
  Serial.println(WiFi.localIP());

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

  // A gesture node has no live readings to advertise, only its identity.
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
  webSocket.loop();
  handleUdpDiscovery();
}

void transportPublish(const char* gesture) {
  if (webSocket.connectedClients() == 0) return;
  StaticJsonDocument<64> doc;
  doc["gesture"] = gesture;
  String out;
  serializeJson(doc, out);
  webSocket.broadcastTXT(out);
  Serial.println(out);
}

#else
// ============================================================
//  BLE TRANSPORT (notify per gesture)
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

// Client must write the shared API key here before gestures are served.
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

// Never serve the cached last gesture to an unauthorized client.
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

void transportPublish(const char* gesture) {
  if (!deviceConnected || !authed) return;
  StaticJsonDocument<64> doc;
  doc["gesture"] = gesture;
  String out;
  serializeJson(doc, out);
  dataChar->setValue(out.c_str());
  dataChar->notify();
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  Wire.begin();
  if (paj7620Init() != 0) {
    Serial.println("Error: PAJ7620 not found!");
    while (1) delay(1000);
  }

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();

  const char* gesture = readGesture();
  if (gesture != nullptr) {
    transportPublish(gesture);
  }
  delay(1);
}

// ============================================================
// Read one gesture from the PAJ7620, or nullptr if none.
//
// A primary swipe may be immediately followed by its reverse; we wait one
// entry window and, if the opposite flag fires, report the combined gesture
// (e.g. right then left -> "rightLeft").
// ============================================================
const char* readGesture() {
  uint8_t data = 0, data2 = 0;
  paj7620ReadReg(0x43, 1, &data);

  switch (data) {
    case GES_RIGHT_FLAG:
      delay(GES_ENTRY_TIME);
      paj7620ReadReg(0x43, 1, &data2);
      return (data2 == GES_LEFT_FLAG) ? "rightLeft" : "right";
    case GES_LEFT_FLAG:
      delay(GES_ENTRY_TIME);
      paj7620ReadReg(0x43, 1, &data2);
      return (data2 == GES_RIGHT_FLAG) ? "leftRight" : "left";
    case GES_UP_FLAG:
      delay(GES_ENTRY_TIME);
      paj7620ReadReg(0x43, 1, &data2);
      return (data2 == GES_DOWN_FLAG) ? "upDown" : "up";
    case GES_DOWN_FLAG:
      delay(GES_ENTRY_TIME);
      paj7620ReadReg(0x43, 1, &data2);
      return (data2 == GES_UP_FLAG) ? "downUp" : "down";
    case GES_FORWARD_FLAG:
      delay(GES_ENTRY_TIME);
      paj7620ReadReg(0x43, 1, &data2);
      return (data2 == GES_BACKWARD_FLAG) ? "forwardBackward" : "forward";
    case GES_BACKWARD_FLAG:
      delay(GES_ENTRY_TIME);
      paj7620ReadReg(0x43, 1, &data2);
      return (data2 == GES_FORWARD_FLAG) ? "backwardForward" : "backward";
    case GES_CLOCKWISE_FLAG:
      return "clockwise";
    case GES_COUNT_CLOCKWISE_FLAG:
      return "antiClockwise";
    default:
      paj7620ReadReg(0x44, 1, &data);
      if (data == GES_WAVE_FLAG) return "wave";
      return nullptr;
  }
}
