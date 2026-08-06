/*
 * Sensor Tester LED Node — ESP32 + LED (+ optional push button)
 *
 * Implements the *actuator* variant of the Sensor Tester Sensor Interface.
 * This is the only node that talks in both directions: the app sends a
 * switch command and the node reports the resulting state back, because the
 * LED can also be toggled by a push button wired to the node itself. The
 * node therefore owns the state and the app only ever renders what the node
 * last reported — a command the node never received cannot leave the app
 * showing a lit LED that is in fact dark.
 *
 * Over Wi-Fi both directions are JSON messages on the WebSocket:
 *
 *     app -> node   {"led":true}     switch on
 *                   {"led":false}    switch off
 *                   {"toggle":true}  flip
 *     node -> app   {"led":true|false}   current state (on connect and on
 *                                        every change, whatever caused it)
 *
 * Over BLE the state is a notify on the data characteristic carrying the same
 * JSON, and the command is a short binary write to the command
 * characteristic (BLE writes are binary-safe, so a switch costs two bytes
 * instead of a JSON document):
 *
 *     0x01 0x00   switch off
 *     0x01 0x01   switch on
 *     0x02        flip
 *
 * The push button is optional: set BUTTON_PIN to -1 for an LED-only node.
 * See ../../python/led/sensor_node.py for the same logic in Python.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app writes the API key to
 *                    AUTH_CHAR_UUID, subscribes to DATA_CHAR_UUID for the
 *                    state and writes commands to CMD_CHAR_UUID.
 *
 * Required Libraries:
 *   ArduinoJson, and for TRANSPORT_WIFI: WiFi, WiFiUdp,
 *   WebSockets (by Markus Sattler / Links2004).
 *   (BLE uses the ESP32 core's built-in BLE stack.)
 */

// ============================================================
//  HARDWARE CONFIGURATION
// ============================================================
// GPIO the LED signal (yellow Grove wire) is connected to.
const int LED_PIN = 5;
// true: the LED lights when the pin is driven LOW (some breakout boards and
// the built-in LED of several ESP32 dev boards are wired that way);
// false: it lights when the pin is driven HIGH (plain Grove/BakeBit LED).
const bool LED_ACTIVE_LOW = false;

// GPIO of the optional local push button, or -1 for an LED-only node.
// The button toggles the LED and the new state is pushed to the app.
const int BUTTON_PIN = 4;
// true: the line idles HIGH and goes LOW when pressed (Grove/BakeBit button);
// false: the line idles LOW and goes HIGH when pressed. setup() enables the
// matching internal pull (pull-up / pull-down) so the idle level is defined.
// (BUTTON_PIN must be a pin with internal pulls — the input-only GPIO34-39
// have none.)
const bool BUTTON_ACTIVE_LOW = true;

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
  #define CMD_CHAR_UUID  "d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f"
#endif

const char* SENSOR_NAME = "LED";

// Command opcodes (see the header comment).
const uint8_t CMD_SET    = 0x01;
const uint8_t CMD_TOGGLE = 0x02;

// Debounce window: the button must be stable this long before a press counts.
const unsigned long DEBOUNCE_MS = 30;

// Current LED state, the single source of truth this node publishes.
bool gLedOn = false;

// Button debounce state.
int gLastReading = -1;
unsigned long gLastChangeMs = 0;
bool gPressed = false;      // last debounced button state
bool gHasReading = false;   // whether gPressed has been established yet

void transportPublish(bool on);

// Returns the state payload string ({"led":true|false}).
String ledJson(bool on) {
  StaticJsonDocument<32> doc;
  doc["led"] = on;
  String out;
  serializeJson(doc, out);
  return out;
}

// Drive the LED pin and record the state, without publishing. Used at boot,
// before a transport exists to publish through.
void writeLed(bool on) {
  gLedOn = on;
  digitalWrite(LED_PIN, LED_ACTIVE_LOW ? (on ? LOW : HIGH)
                                       : (on ? HIGH : LOW));
}

// Switch the LED and publish the new state. Publishing unconditionally
// (rather than only on a change) keeps a redundant command from leaving a
// client that guessed wrong without a correction.
void applyLed(bool on) {
  writeLed(on);
  transportPublish(on);
}

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
// ============================================================
//  WIFI TRANSPORT (WebSocket commands + state push, UDP discovery)
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
  StaticJsonDocument<128> doc;
  // Zero-copy parse; the WebSockets library null-terminates text frames.
  DeserializationError error = deserializeJson(doc, (char*)payload, len);
  if (error) {
    Serial.println("Ignoring malformed command");
    return;
  }

  if (doc["toggle"] == true) {
    Serial.println("Command: toggle");
    applyLed(!gLedOn);
    return;
  }

  if (!doc["led"].is<bool>()) {
    Serial.println("Ignoring command without a boolean 'led'");
    return;
  }
  bool on = doc["led"];
  Serial.printf("Command: led %s\n", on ? "on" : "off");
  applyLed(on);
}

void webSocketEvent(uint8_t num, WStype_t type, uint8_t* payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED: {
      Serial.printf("[%u] Client connected\n", num);
      // Send the current state so the app shows the LED as it actually is
      // rather than waiting for the next change. sendTXT takes String& (an
      // lvalue), so hold it in a variable.
      String out = ledJson(gLedOn);
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

  // An LED node has no readings to advertise, only its identity.
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

void transportPublish(bool on) {
  if (webSocket.connectedClients() == 0) return;
  String out = ledJson(on);
  webSocket.broadcastTXT(out);
  Serial.println(out);
}

#else
// ============================================================
//  BLE TRANSPORT (binary commands in, state notify out)
// ============================================================
BLECharacteristic* dataChar = nullptr;
bool deviceConnected = false;
bool authed = false;

// Handle one command packet written to CMD_CHAR_UUID.
void handleBleCommand(const uint8_t* packet, size_t len) {
  if (len == 0) {
    Serial.println("Ignoring empty command packet");
    return;
  }

  switch (packet[0]) {
    case CMD_TOGGLE:
      Serial.println("Command: toggle");
      applyLed(!gLedOn);
      return;

    case CMD_SET:
      if (len < 2) {
        Serial.println("Ignoring set without a state byte");
        return;
      }
      Serial.printf("Command: led %s\n", packet[1] ? "on" : "off");
      applyLed(packet[1] != 0);
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
    authed = false;
    server->getAdvertising()->start();  // allow the next client to find us
  }
};

// Client must write the shared API key here before the LED can be switched.
class AuthCallbacks : public BLECharacteristicCallbacks {
  void onWrite(BLECharacteristic* characteristic) override {
    String val = characteristic->getValue();
    while (val.length() > 0 && (val[val.length() - 1] == '\0' || val[val.length() - 1] == '\r' || val[val.length() - 1] == '\n')) {
      val.remove(val.length() - 1);
    }
    authed = (val == API_KEY);
    Serial.println(authed ? "Client authorized" : "Bad API key");
    // Push the current state right after a successful authorization.
    if (authed) {
      notifySensorJson(dataChar, ledJson(gLedOn));
    }
  }
};

// Serve the current LED state only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? ledJson(gLedOn).c_str() : "{}");
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

  // Accept both write flavours, like the display node: a client that can use
  // write-without-response gets a slightly snappier switch.
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

void transportPublish(bool on) {
  if (!deviceConnected || !authed) return;
  String out = ledJson(on);
  notifySensorJson(dataChar, out);
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester LED Node ---");

  pinMode(LED_PIN, OUTPUT);
  // Start dark, driving the pin explicitly so the reported state and the
  // physical LED agree from the first moment. Not applyLed(): no transport
  // is up yet to publish through.
  writeLed(false);

  if (BUTTON_PIN >= 0) {
    // Give the idle state a defined level via an internal pull resistor:
    //  - active-low buttons idle HIGH -> pull-up
    //  - active-high buttons idle LOW -> pull-down
    pinMode(BUTTON_PIN, BUTTON_ACTIVE_LOW ? INPUT_PULLUP : INPUT_PULLDOWN);
  }

  transportSetup();

  Serial.print("Actuator: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
// Read the button with debounce; toggle the LED on each press (not on
// release, so one press is one toggle).
// ============================================================
void pollButton() {
  if (BUTTON_PIN < 0) return;

  int reading = digitalRead(BUTTON_PIN);
  if (reading != gLastReading) {
    gLastReading = reading;
    gLastChangeMs = millis();
    return;
  }
  if (millis() - gLastChangeMs < DEBOUNCE_MS) return;

  bool pressed = BUTTON_ACTIVE_LOW ? (reading == LOW) : (reading == HIGH);
  if (!gHasReading) {
    // The very first stable reading only establishes the idle level; treating
    // it as an edge would toggle the LED at boot whenever the button happens
    // to be held.
    gPressed = pressed;
    gHasReading = true;
    return;
  }
  if (pressed == gPressed) return;

  gPressed = pressed;
  if (pressed) {
    Serial.println("Button pressed: toggling LED");
    applyLed(!gLedOn);
  }
}

// ============================================================
void loop() {
  transportLoop();
  pollButton();
  delay(1);
}
