/*
 * Sensor Playground Speaker Node — ESP32 + Grove Speaker
 *
 * Implements the *actuator* variant of the Sensor Playground Sensor Interface.
 * The Grove Speaker is a small amplified loudspeaker on a digital pin,
 * driven with a square wave of the desired pitch (LEDC peripheral). Like
 * the LED the node talks in both directions: the app asks for a tone, the
 * built-in melody, or silence, and the node reports what is *actually*
 * sounding — a tone ends on its own when its duration runs out, so the app
 * follows the node's reports rather than its own taps.
 *
 * Over Wi-Fi both directions are JSON messages on the WebSocket:
 *
 *     app -> node   {"tone":{"freq":440,"ms":400}}   play one tone
 *                   {"melody":true}                  play the built-in melody
 *                   {"stop":true}                    silence
 *     node -> app   {"freq":440}                     sounding (on connect and
 *                   {"freq":0}                       on every change)
 *
 * Over BLE the state is a notify on the data characteristic carrying the
 * same JSON, and a command is a short binary write to the command
 * characteristic:
 *
 *     0x01 <freq:u16 big-endian> <ms:u16 big-endian>   play one tone
 *     0x02                                             play the melody
 *     0x03                                             silence
 *
 * See ../../python/speaker/sensor_node.py for the same logic in Python.
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
 *   (BLE and the LEDC tone output use the ESP32 core; no sensor library.)
 */

// ============================================================
//  HARDWARE CONFIGURATION
// ============================================================
// GPIO the speaker signal (yellow Grove wire) is connected to.
const int SPEAKER_PIN = 5;

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

  // Shared Sensor Playground GATT contract (must match the app's BleUuids).
  #define SERVICE_UUID   "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
  #define DATA_CHAR_UUID "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
  #define AUTH_CHAR_UUID "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
  #define CMD_CHAR_UUID  "d1a51b00-0004-4a7e-9b3c-0a1b2c3d4e5f"
#endif

const char* SENSOR_NAME = "SPEAKER";

// Command opcodes (see the header comment).
const uint8_t CMD_TONE   = 0x01;
const uint8_t CMD_MELODY = 0x02;
const uint8_t CMD_STOP   = 0x03;

// Accepted tone range; anything else is ignored as noise.
const uint16_t MIN_FREQ_HZ = 20;
const uint16_t MAX_FREQ_HZ = 20000;

// One note of the built-in melody.
struct ToneStep {
  uint16_t freq;
  uint16_t ms;
};

// A little C-major fanfare: C E G C' G E C.
const ToneStep MELODY[] = {
  {262, 250}, {330, 250}, {392, 250}, {523, 350},
  {392, 250}, {330, 250}, {262, 500},
};
const size_t MELODY_LENGTH = sizeof(MELODY) / sizeof(MELODY[0]);

// Frequency currently sounding (0 = silent) — the single source of truth
// this node publishes.
uint16_t gFreq = 0;
// When the current tone or melody step ends.
unsigned long gToneEndMs = 0;
// Next melody step to play, or -1 outside a melody.
int gMelodyIndex = -1;

void transportPublish();

// Returns the state payload string ({"freq":440}).
String speakerJson() {
  StaticJsonDocument<32> doc;
  doc["freq"] = gFreq;
  String out;
  serializeJson(doc, out);
  return out;
}

// Drive the LEDC output and publish the new state. Publishing
// unconditionally keeps a redundant command from leaving a client that
// guessed wrong without a correction.
void applyFreq(uint16_t freq) {
  gFreq = freq;
  ledcWriteTone(SPEAKER_PIN, freq);
  transportPublish();
}

// Play one tone for [ms] milliseconds, cancelling any melody.
void startTone(uint16_t freq, uint16_t ms) {
  if (freq < MIN_FREQ_HZ || freq > MAX_FREQ_HZ || ms == 0) {
    Serial.printf("Ignoring tone %u Hz / %u ms\n", freq, ms);
    return;
  }
  gMelodyIndex = -1;
  gToneEndMs = millis() + ms;
  Serial.printf("Tone: %u Hz for %u ms\n", freq, ms);
  applyFreq(freq);
}

// Start the built-in melody from its first note.
void startMelody() {
  Serial.println("Melody");
  gMelodyIndex = 0;
  gToneEndMs = millis() + MELODY[0].ms;
  applyFreq(MELODY[0].freq);
}

// Silence the speaker, whatever it is playing.
void stopPlayback() {
  Serial.println("Stop");
  gMelodyIndex = -1;
  applyFreq(0);
}

// Advance playback: end a finished tone, or step through the melody. Runs
// from loop(), so tones end on time without blocking the transports.
void updatePlayback() {
  if (gFreq == 0 || millis() < gToneEndMs) return;

  if (gMelodyIndex >= 0 && (size_t)(gMelodyIndex + 1) < MELODY_LENGTH) {
    gMelodyIndex++;
    gToneEndMs = millis() + MELODY[gMelodyIndex].ms;
    applyFreq(MELODY[gMelodyIndex].freq);
  } else {
    gMelodyIndex = -1;
    applyFreq(0);
  }
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

  if (doc["stop"] == true) {
    stopPlayback();
    return;
  }
  if (doc["melody"] == true) {
    startMelody();
    return;
  }
  if (doc["tone"].is<JsonObject>()) {
    startTone(doc["tone"]["freq"] | 0, doc["tone"]["ms"] | 0);
    return;
  }
  Serial.println("Ignoring unknown command");
}

void webSocketEvent(uint8_t num, WStype_t type, uint8_t* payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED: {
      Serial.printf("[%u] Client connected\n", num);
      // Send the current state so the app shows the speaker as it actually
      // is. sendTXT takes String& (an lvalue), so hold it in a variable.
      String out = speakerJson();
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

  // A speaker node has no readings to advertise, only its identity.
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
  String out = speakerJson();
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
    case CMD_STOP:
      stopPlayback();
      return;

    case CMD_MELODY:
      startMelody();
      return;

    case CMD_TONE: {
      if (len < 5) {
        Serial.println("Ignoring tone without frequency and duration");
        return;
      }
      uint16_t freq = ((uint16_t)packet[1] << 8) | packet[2];
      uint16_t ms = ((uint16_t)packet[3] << 8) | packet[4];
      startTone(freq, ms);
      return;
    }

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

// Client must write the shared API key here before the speaker plays.
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
      notifySensorJson(dataChar, speakerJson());
    }
  }
};

// Serve the current state only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? speakerJson().c_str() : "{}");
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

  // Accept both write flavours, like the LED node.
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
  String out = speakerJson();
  notifySensorJson(dataChar, out);
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Playground Speaker Node ---");

  // Attach the pin to an LEDC channel; ledcWriteTone then reprograms the
  // frequency per note, and frequency 0 silences the output.
  if (!ledcAttach(SPEAKER_PIN, 1000, 10)) {
    Serial.println("Error: LEDC attach failed!");
    while (1) delay(1000);
  }
  ledcWriteTone(SPEAKER_PIN, 0);

  transportSetup();

  Serial.print("Actuator: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();
  updatePlayback();
  delay(1);
}
