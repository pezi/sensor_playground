/*
 * Sensor Tester Sensor Node — ESP32 Digital Contact Sensor (push)
 *
 * One generic *push* node for the simple digital Grove/BakeBit sensors that
 * only have two states (active / inactive) and react to an event:
 *
 *   BUTTON     — Grove/BakeBit button      (pressed)
 *   HALL       — Grove Hall sensor         (magnetic field present)
 *   MAGSWITCH  — Grove magnetic switch     (reed switch closed)
 *   PIR        — Grove PIR motion sensor   (motion detected)
 *   VIBRATION  — Grove vibration sensor    (SW-420, vibration)
 *   LINEFINDER — Grove Line Finder         (dark line under the sensor)
 *
 * Like the gesture/distance nodes this is push, not poll: the node reads a
 * (debounced) GPIO and sends one JSON message whenever the state changes:
 *
 *     {"active": true}    sensor triggered
 *     {"active": false}   sensor released
 *
 * The original dart_periphery examples toggle a local LED; this node ignores
 * that and instead reports the state to the Sensor Tester app.
 *
 * Configure the three defines below for your sensor (see the table). The app
 * recognises the SENSOR_NAME and opens the generic trigger screen.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app writes the API key to
 *                    AUTH_CHAR_UUID, then subscribes to DATA_CHAR_UUID; each
 *                    state change arrives as one notify.
 *
 * Required Libraries:
 *   ArduinoJson, and for TRANSPORT_WIFI: WiFi, WiFiUdp,
 *   WebSockets (by Markus Sattler / Links2004).
 *   (BLE uses the ESP32 core's built-in BLE stack.)
 */

// ============================================================
//  SENSOR CONFIGURATION — set these three for your sensor
// ============================================================
//
//  | Sensor     | SENSOR_NAME   | ACTIVE_LOW |
//  |------------|---------------|------------|
//  | Button     | "BUTTON"      | true       |
//  | Hall       | "HALL"        | true       |
//  | MagSwitch  | "MAGSWITCH"   | false      |
//  | PIR        | "PIR"         | false      |
//  | Vibration  | "VIBRATION"   | true       |
//  | LineFinder | "LINEFINDER"  | false *    |
//
//  * The Line Finder's output polarity differs between board revisions, and
//    its on-board potentiometer sets the black/white threshold rather than the
//    direction. Hold the sensor over the line and check the app: if it reads
//    "Line detected" over the bright surface instead, flip ACTIVE_LOW.
//
#define SENSOR_NAME "HALL"
// true: the line idles HIGH and goes LOW when active (button, hall, vibration);
// false: the line idles LOW and goes HIGH when active (magnetic switch, PIR).
// setup() enables the matching internal pull (pull-up / pull-down) so the idle
// state is defined even for sensors that only drive one edge (e.g. the reed
// switch, which connects the line to VCC only while the magnet holds it shut).
const bool ACTIVE_LOW = true;
// GPIO the Grove signal (yellow) wire is connected to.
const int INPUT_PIN = 4;

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

// Debounce window: the line must be stable this long before a change counts.
const unsigned long DEBOUNCE_MS = 30;

// Debounce state.
int gLastReading = -1;
unsigned long gLastChangeMs = 0;
bool gActive = false;          // last published (debounced) active state
bool gHasState = false;        // whether gActive has been established yet

void transportPublish(bool active);
void transportNotifyNewClient();

// Returns the active payload string ({"active":true|false}).
String activeJson(bool active) {
  StaticJsonDocument<32> doc;
  doc["active"] = active;
  String out;
  serializeJson(doc, out);
  return out;
}

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
      // Send the current state so the app shows it without waiting for a
      // change. sendTXT takes String& (an lvalue), so hold it in a variable.
      if (gHasState) {
        String out = activeJson(gActive);
        webSocket.sendTXT(num, out);
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

  // A contact node has no live readings to advertise, only its identity.
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

void transportPublish(bool active) {
  if (webSocket.connectedClients() == 0) return;
  String out = activeJson(active);
  webSocket.broadcastTXT(out);
  Serial.println(out);
}

#else
// ============================================================
//  BLE TRANSPORT (notify per state change)
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

// Client must write the shared API key here before data is served.
class AuthCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    String val = characteristic->getValue();
    while (val.length() > 0 && (val[val.length() - 1] == '\0' || val[val.length() - 1] == '\r' || val[val.length() - 1] == '\n')) {
      val.remove(val.length() - 1);
    }
    authed = (val == API_KEY);
    Serial.println(authed ? "Client authorized" : "Bad API key");
    // Push the current state right after a successful authorization.
    if (authed && gHasState) {
      dataChar->setValue(activeJson(gActive).c_str());
      dataChar->notify();
    }
  }
};

// Serve the current contact state only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    if (authed && gHasState) {
      characteristic->setValue(activeJson(gActive).c_str());
    } else {
      characteristic->setValue("{}");
    }
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

void transportPublish(bool active) {
  if (!deviceConnected || !authed) return;
  String out = activeJson(active);
  dataChar->setValue(out.c_str());
  dataChar->notify();
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  // Give the idle state a defined level via an internal pull resistor:
  //  - active-low sensors idle HIGH  -> pull-up
  //  - active-high sensors idle LOW   -> pull-down
  // The pull-down matters for reed-based sensors like the Grove Magnetic
  // Switch: it only connects the line to VCC when the magnet closes it and
  // leaves it floating otherwise, so without a pull-down the "released" state
  // never reads a clean LOW and no change is ever reported. (INPUT_PIN must be
  // a pin with internal pulls — the input-only GPIO34-39 have none.)
  pinMode(INPUT_PIN, ACTIVE_LOW ? INPUT_PULLUP : INPUT_PULLDOWN);

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
// Read the input pin with debounce; publish on a stable state change.
// ============================================================
void pollInput() {
  int reading = digitalRead(INPUT_PIN);
  if (reading != gLastReading) {
    gLastReading = reading;
    gLastChangeMs = millis();
    return;
  }
  if (millis() - gLastChangeMs < DEBOUNCE_MS) return;

  bool active = ACTIVE_LOW ? (reading == LOW) : (reading == HIGH);
  if (!gHasState || active != gActive) {
    gActive = active;
    gHasState = true;
    transportPublish(active);
  }
}

// ============================================================
void loop() {
  transportLoop();
  pollInput();
  delay(1);
}
