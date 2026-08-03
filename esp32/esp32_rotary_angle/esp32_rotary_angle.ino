/*
 * Sensor Tester Sensor Node — ESP32 + Grove Rotary Angle Sensor (analog, push)
 *
 * Reads a Grove Rotary Angle Sensor — a 10 kOhm potentiometer with 300° of
 * mechanical travel — on an ADC pin and reports the knob position.
 * https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/
 *
 * Unlike the light sensor (also analog, but polled over HTTPS every few
 * seconds) this is a *push* node: the app draws a needle that tracks the
 * knob, so a reading that is seconds old is useless. The node samples the
 * ADC continuously and sends a message whenever the knob has moved further
 * than ADC_DEADBAND, plus the current position once per client connect:
 *
 *     {"adc":2048,"adcMax":4095,"angle":150.1,"angleMax":300.0}
 *
 * ADC RANGE — why adcMax is in every message
 * ------------------------------------------
 * The converter's width belongs to the board doing the reading, not to the
 * knob, and the boards this project supports do not agree:
 *
 *     ESP32 (this sketch)          12-bit    0 - 4095
 *     Seeed Grove Base Hat         12-bit    0 - 4095
 *     FriendlyARM NanoHat Hub      10-bit    0 - 1023   (BakeBit firmware)
 *     Seeed GrovePi+               10-bit    0 - 1023
 *
 * A bare "512" is therefore ambiguous — half travel on a NanoHat, an eighth
 * on an ESP32. Sending the full-scale value alongside the count lets the app
 * scale the dial to whichever node it is talking to instead of assuming one
 * of them. `angle`/`angleMax` carry the same position expressed in degrees,
 * so the app never needs to know the knob's mechanical travel either.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — WebSocket server on port 9132 (ws://, X-Api-Key header)
 *                    + UDP discovery on port 9133 (SENSOR_TESTER contract)
 *   TRANSPORT_BLE  — BLE GATT service; the app writes the API key to
 *                    AUTH_CHAR_UUID, then subscribes to DATA_CHAR_UUID; each
 *                    movement arrives as one notify.
 *
 * Required Libraries:
 *   ArduinoJson, and for TRANSPORT_WIFI: WiFi, WiFiUdp,
 *   WebSockets (by Markus Sattler / Links2004).
 *   (BLE uses the ESP32 core's built-in BLE stack.
 *    No sensor library — the potentiometer is read with analogRead.)
 */

// ============================================================
//  HARDWARE CONFIGURATION
// ============================================================
// ADC pin the Grove signal (yellow) wire is connected to.
// GPIO 34 is input-only and on ADC1, which stays usable with Wi-Fi enabled.
// (ADC2 pins stop working the moment the Wi-Fi radio is on.)
const int ROTARY_PIN = 34;

// The ESP32's ADC is 12-bit; setup() configures it to match.
const int ADC_BITS = 12;
const int ADC_MAX  = (1 << ADC_BITS) - 1;  // 4095

// Mechanical travel of the knob, end to end. 300° for the Grove sensor.
const float ANGLE_MAX = 300.0f;

// How far the raw count must move before a new message goes out. The ESP32's
// ADC is noisy enough to jitter by a handful of counts with the knob at rest,
// which would otherwise flood the link with meaningless traffic.
const int ADC_DEADBAND = 24;  // ~0.6 % of full scale

// Shortest gap between two messages. The knob can be swept faster than the
// app can draw, so this caps the rate without adding lag you can feel.
const unsigned long MIN_PUBLISH_INTERVAL_MS = 40;

// Readings averaged per sample, to take the edge off the ADC noise.
const int OVERSAMPLE = 8;

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

const char* SENSOR_NAME = "ROTARY";

// Last published position, and whether anything has been published yet.
int gLastPublishedAdc = 0;
bool gHasPublished = false;
unsigned long gLastPublishMs = 0;

void transportPublish(int adc);

// Average OVERSAMPLE conversions into one raw count.
int readAdc() {
  long total = 0;
  for (int i = 0; i < OVERSAMPLE; i++) {
    total += analogRead(ROTARY_PIN);
  }
  int value = (int)(total / OVERSAMPLE);
  // analogRead cannot exceed ADC_MAX at this resolution, but clamping keeps
  // the contract with the app explicit: adc is never above adcMax.
  if (value < 0) value = 0;
  if (value > ADC_MAX) value = ADC_MAX;
  return value;
}

// Build the position payload. The scale travels with every message — see the
// ADC RANGE note in the header.
String rotaryJson(int adc) {
  StaticJsonDocument<128> doc;
  doc["adc"]      = adc;
  doc["adcMax"]   = ADC_MAX;
  doc["angle"]    = (float)adc / (float)ADC_MAX * ANGLE_MAX;
  doc["angleMax"] = ANGLE_MAX;
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
    case WStype_CONNECTED: {
      Serial.printf("[%u] Client connected\n", num);
      // Send the current position so the needle starts where the knob is
      // instead of at zero. sendTXT takes String& (an lvalue), so hold it in
      // a variable.
      String out = rotaryJson(gHasPublished ? gLastPublishedAdc : readAdc());
      webSocket.sendTXT(num, out);
      break;
    }
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

  // Like the other push nodes, discovery advertises identity only; the
  // position arrives on the WebSocket.
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

void transportPublish(int adc) {
  if (webSocket.connectedClients() == 0) return;
  String out = rotaryJson(adc);
  webSocket.broadcastTXT(out);
  Serial.println(out);
}

#else
// ============================================================
//  BLE TRANSPORT (notify per movement)
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
    // Push the current position right after a successful authorization.
    if (authed) {
      dataChar->setValue(rotaryJson(readAdc()).c_str());
      dataChar->notify();
    }
  }
};

// Serve the current position only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? rotaryJson(readAdc()).c_str() : "{}");
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

void transportPublish(int adc) {
  if (!deviceConnected || !authed) return;
  String out = rotaryJson(adc);
  dataChar->setValue(out.c_str());
  dataChar->notify();
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  analogReadResolution(ADC_BITS);  // 0-4095
  // Full-scale attenuation: the Grove sensor swings the whole 0-3.3 V rail,
  // and without this the reading saturates well before the end stop.
  analogSetPinAttenuation(ROTARY_PIN, ADC_11db);

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
  Serial.printf("ADC range: 0-%d (%d-bit), travel %.0f deg\n",
                ADC_MAX, ADC_BITS, ANGLE_MAX);
}

// ============================================================
// Sample the knob; publish when it has moved past the deadband.
// ============================================================
void pollRotary() {
  if (millis() - gLastPublishMs < MIN_PUBLISH_INTERVAL_MS) return;

  int adc = readAdc();
  if (gHasPublished && abs(adc - gLastPublishedAdc) < ADC_DEADBAND) {
    // Still within the noise band around the last published position; also
    // pin the ends, so a knob turned fully still reports exactly 0 / ADC_MAX
    // rather than stopping a deadband short of it.
    bool atEnd = (adc == 0 || adc == ADC_MAX) && adc != gLastPublishedAdc;
    if (!atEnd) return;
  }

  gLastPublishedAdc = adc;
  gHasPublished = true;
  gLastPublishMs = millis();
  transportPublish(adc);
}

// ============================================================
void loop() {
  transportLoop();
  pollRotary();
  delay(1);
}
