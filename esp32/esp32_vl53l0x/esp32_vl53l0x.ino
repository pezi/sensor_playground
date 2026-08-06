/*
 * Sensor Tester Sensor Node — ESP32 + VL53L0X (Time-of-Flight)
 *
 * Implements a *push* variant of the Sensor Tester Sensor Interface. Like the
 * gesture node, a distance sensor is event-driven: the node measures
 * continuously and pushes one JSON message whenever the distance changes
 * (or at least once per second as a heartbeat):
 *
 *     {"distance": 234}      // millimeters
 *     {"distance": null}     // target out of range
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then subscribes to
 *                    DATA_CHAR_UUID; each measurement arrives as one notify.
 *
 * Required Libraries:
 *   ArduinoJson, Wire, Adafruit_VL53L0X,
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
#include <Adafruit_VL53L0X.h>
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

const char* SENSOR_NAME = "VL53L0X";

Adafruit_VL53L0X lox;

// --- Publish policy ---
// Measure every MEASURE_INTERVAL_MS; publish when the distance moved by at
// least MIN_DELTA_MM, the in/out-of-range state flipped, or HEARTBEAT_MS
// elapsed since the last publish.
const unsigned long MEASURE_INTERVAL_MS = 100;
const unsigned long HEARTBEAT_MS = 1000;
const int MIN_DELTA_MM = 3;

uint16_t lastSentMm = 0;
bool lastSentValid = false;
unsigned long lastMeasureMs = 0;
unsigned long lastPublishMs = 0;
bool everPublished = false;

void transportPublish(bool valid, uint16_t mm);

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

  // A push node has no pollable readings to advertise, only its identity.
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

void transportPublish(bool valid, uint16_t mm) {
  if (webSocket.connectedClients() == 0) return;
  StaticJsonDocument<64> doc;
  if (valid) {
    doc["distance"] = mm;
  } else {
    doc["distance"] = nullptr;
  }
  String out;
  serializeJson(doc, out);
  webSocket.broadcastTXT(out);
}

#else
// ============================================================
//  BLE TRANSPORT (notify per measurement)
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

// Client must write the shared API key here before measurements are served.
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

// Never serve the cached last measurement to an unauthorized client.
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

void transportPublish(bool valid, uint16_t mm) {
  if (!deviceConnected || !authed) return;
  StaticJsonDocument<64> doc;
  if (valid) {
    doc["distance"] = mm;
  } else {
    doc["distance"] = nullptr;
  }
  String out;
  serializeJson(doc, out);
  notifySensorJson(dataChar, out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  Wire.begin();
  if (!lox.begin()) {
    Serial.println("Error: VL53L0X not found!");
    while (1) delay(1000);
  }

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();

  if (millis() - lastMeasureMs < MEASURE_INTERVAL_MS) {
    delay(1);
    return;
  }
  lastMeasureMs = millis();

  VL53L0X_RangingMeasurementData_t measure;
  lox.rangingTest(&measure, false);
  bool valid = (measure.RangeStatus != 4);  // 4 = phase failure / no target
  uint16_t mm = measure.RangeMilliMeter;

  bool changed = !everPublished ||
                 valid != lastSentValid ||
                 (valid && abs((int)mm - (int)lastSentMm) >= MIN_DELTA_MM);
  if (changed || millis() - lastPublishMs >= HEARTBEAT_MS) {
    transportPublish(valid, mm);
    everPublished = true;
    lastSentValid = valid;
    lastSentMm = mm;
    lastPublishMs = millis();
  }
}
