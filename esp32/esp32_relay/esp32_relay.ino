/*
 * Sensor Playground Relay Node — ESP32 + Grove SPDT Relay (1 / 2 / 4 channels)
 *
 * Implements the *actuator* variant of the Sensor Playground Sensor Interface.
 * Like the LED node it talks in both directions: the app switches a channel
 * and the node reports the resulting state of every channel back. The node
 * owns the state and the app only ever renders what the node last reported —
 * a command the node never received cannot leave the app showing a closed
 * contact that is in fact open.
 *
 * How many channels this board carries is part of the node's configuration
 * (see RELAY_INTERFACE below) and travels with every message, so the app
 * draws exactly one button per relay instead of guessing:
 *
 *     app -> node   {"ch":0,"on":true}      switch channel 0 (zero-based)
 *                   {"ch":0,"toggle":true}  flip channel 0
 *                   {"all":false}           switch every channel off
 *     node -> app   {"channels":2,"relay":[true,false]}
 *                                           state of the whole board (on
 *                                           connect and after every change)
 *
 * Over BLE the state is a notify on the data characteristic carrying the same
 * JSON, and the commands are short binary writes to the command
 * characteristic (BLE writes are binary-safe, so a switch costs three bytes
 * instead of a JSON document):
 *
 *     0x01 <ch> 0x00   switch channel <ch> off
 *     0x01 <ch> 0x01   switch channel <ch> on
 *     0x02 <ch>        flip channel <ch>
 *     0x03 0x00|0x01   switch every channel at once
 *
 * See ../../python/relay/sensor_node.py for the same logic in Python.
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
 *   (BLE uses the ESP32 core's built-in BLE stack; the I2C board uses the
 *   core's built-in Wire library — the Seeed relay library is not needed,
 *   the two-byte command is issued directly.)
 */

// ============================================================
//  HARDWARE CONFIGURATION
// ============================================================
// How the relay board is wired. The Grove 1- and 2-channel SPDT modules take
// one digital pin per channel, and so do the common 4-channel "4 Channel 5V
// Relay Module" boards (SunFounder & clones) — list one pin per channel for
// any of them. Only the Grove 4-channel module is an I2C device, whose
// on-board MCU switches the coils.
#define RELAY_IFACE_GPIO 0
#define RELAY_IFACE_I2C  1
#define RELAY_INTERFACE  RELAY_IFACE_GPIO

// --- GPIO interface (1 to 8 channels on plain digital pins) ---
// One GPIO per channel, in channel order: the first entry is "Relay 1" in
// the app, and the length of this list *is* the channel count. A 1-channel
// board simply lists one pin; a 4-channel SunFounder-style board lists four:
//     const int RELAY_PINS[] = {5};                // 1 channel
//     const int RELAY_PINS[] = {5, 18};            // 2 channels (Grove)
//     const int RELAY_PINS[] = {5, 18, 19, 21};    // 4 channels (IN1..IN4)
const int RELAY_PINS[] = {5, 18};
// true: the coil is energized when the pin is driven LOW — how most of the
// cheap 4-channel "relay module" boards are wired; false: energized when
// driven HIGH, which is how the Grove SPDT modules work.
//
// Get this wrong and the load sits energized while the app shows "off": the
// sketch opens every channel at boot, so if the relays click closed on reset,
// flip this flag.
const bool RELAY_ACTIVE_LOW = false;

// --- I2C interface (Grove 4-Channel SPDT Relay) ---
// Factory address is 0x11 (0x12 on some batches); the module can be moved
// anywhere in 0x00..0x7F with its "save address" command.
const uint8_t RELAY_I2C_ADDRESS  = 0x11;
// Channels the board carries — 4 for the Grove 4-Channel SPDT Relay.
const int     RELAY_I2C_CHANNELS = 4;
// Command byte of the on-board MCU: 0x10 takes a channel bitmask, bit 0
// being channel 1 (matching Seeed's Multi_Channel_Relay library).
const uint8_t RELAY_I2C_CMD_CHANNEL_CTRL = 0x10;

// --- Transport selection (change this line) ---
#define TRANSPORT_WIFI 0
#define TRANSPORT_BLE  1
#ifndef ACTIVE_TRANSPORT
#define ACTIVE_TRANSPORT TRANSPORT_BLE
#endif

#include <ArduinoJson.h>
#include "secrets.h"

#if RELAY_INTERFACE == RELAY_IFACE_I2C
  #include <Wire.h>
#endif

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

const char* SENSOR_NAME = "RELAY";

// Command opcodes (see the header comment).
const uint8_t CMD_SET    = 0x01;
const uint8_t CMD_TOGGLE = 0x02;
const uint8_t CMD_ALL    = 0x03;

// Channels this node drives, derived from the wiring above so the count is
// configured in exactly one place.
#if RELAY_INTERFACE == RELAY_IFACE_GPIO
const int CHANNEL_COUNT = sizeof(RELAY_PINS) / sizeof(RELAY_PINS[0]);
#else
const int CHANNEL_COUNT = RELAY_I2C_CHANNELS;
#endif

// The I2C module carries up to 8 coils; the bitmask is one byte either way.
const int MAX_CHANNELS = 8;

// Current state of every channel — the single source of truth this node
// publishes.
bool gRelays[MAX_CHANNELS] = {false};

void transportPublish();

// Returns the state payload string ({"channels":n,"relay":[…]}).
String relayJson() {
  StaticJsonDocument<192> doc;
  doc["channels"] = CHANNEL_COUNT;
  JsonArray states = doc.createNestedArray("relay");
  for (int channel = 0; channel < CHANNEL_COUNT; channel++) {
    states.add(gRelays[channel]);
  }
  String out;
  serializeJson(doc, out);
  return out;
}

#if RELAY_INTERFACE == RELAY_IFACE_I2C
// Push all channels to the module at once: it takes a bitmask, so a
// per-channel write would be a read-modify-write of state we already hold.
void writeRelayBoard() {
  uint8_t mask = 0;
  for (int channel = 0; channel < CHANNEL_COUNT; channel++) {
    if (gRelays[channel]) mask |= (1 << channel);
  }
  Wire.beginTransmission(RELAY_I2C_ADDRESS);
  Wire.write(RELAY_I2C_CMD_CHANNEL_CTRL);
  Wire.write(mask);
  if (Wire.endTransmission() != 0) {
    Serial.println("I2C write failed — is the relay board connected?");
  }
}
#else
void writeRelayBoard() {
  for (int channel = 0; channel < CHANNEL_COUNT; channel++) {
    const bool on = gRelays[channel];
    digitalWrite(RELAY_PINS[channel],
                 RELAY_ACTIVE_LOW ? (on ? LOW : HIGH) : (on ? HIGH : LOW));
  }
}
#endif

// Drive the board and record the state, without publishing. Used at boot,
// before a transport exists to publish through.
void writeRelay(int channel, bool on) {
  if (channel < 0 || channel >= CHANNEL_COUNT) return;
  gRelays[channel] = on;
  writeRelayBoard();
}

// Switch one channel and publish the whole board. Publishing unconditionally
// (rather than only on a change) keeps a redundant command from leaving a
// client that guessed wrong without a correction.
void applyRelay(int channel, bool on) {
  if (channel < 0 || channel >= CHANNEL_COUNT) {
    Serial.printf("Ignoring command for channel %d (board has %d)\n",
                  channel, CHANNEL_COUNT);
    return;
  }
  writeRelay(channel, on);
  transportPublish();
}

// Switch every channel at once — one board write, one state message.
void applyAllRelays(bool on) {
  for (int channel = 0; channel < CHANNEL_COUNT; channel++) {
    gRelays[channel] = on;
  }
  writeRelayBoard();
  transportPublish();
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

  if (doc["all"].is<bool>()) {
    bool on = doc["all"];
    Serial.printf("Command: all %s\n", on ? "on" : "off");
    applyAllRelays(on);
    return;
  }

  if (!doc["ch"].is<int>()) {
    Serial.println("Ignoring command without a channel");
    return;
  }
  int channel = doc["ch"];

  if (doc["toggle"] == true) {
    Serial.printf("Command: toggle channel %d\n", channel);
    if (channel >= 0 && channel < CHANNEL_COUNT) {
      applyRelay(channel, !gRelays[channel]);
    } else {
      Serial.printf("Ignoring command for channel %d (board has %d)\n",
                    channel, CHANNEL_COUNT);
    }
    return;
  }

  if (!doc["on"].is<bool>()) {
    Serial.println("Ignoring command without a boolean 'on'");
    return;
  }
  bool on = doc["on"];
  Serial.printf("Command: channel %d %s\n", channel, on ? "on" : "off");
  applyRelay(channel, on);
}

void webSocketEvent(uint8_t num, WStype_t type, uint8_t* payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED: {
      Serial.printf("[%u] Client connected\n", num);
      // Send the current state so the app shows the board as it actually is
      // — and learns its channel count — rather than waiting for a change.
      // sendTXT takes String& (an lvalue), so hold it in a variable.
      String out = relayJson();
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

  // A relay node has no readings to advertise — only its identity and how
  // many channels its board carries.
  StaticJsonDocument<256> doc;
  doc["type"]     = SENSOR_NAME;
  doc["host"]     = HOSTNAME;
  doc["ip"]       = WiFi.localIP().toString();
  doc["port"]     = WS_PORT;
  doc["channels"] = CHANNEL_COUNT;
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
  String out = relayJson();
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
    case CMD_ALL:
      if (len < 2) {
        Serial.println("Ignoring all without a state byte");
        return;
      }
      Serial.printf("Command: all %s\n", packet[1] ? "on" : "off");
      applyAllRelays(packet[1] != 0);
      return;

    case CMD_TOGGLE: {
      if (len < 2) {
        Serial.println("Ignoring toggle without a channel byte");
        return;
      }
      int channel = packet[1];
      Serial.printf("Command: toggle channel %d\n", channel);
      if (channel >= 0 && channel < CHANNEL_COUNT) {
        applyRelay(channel, !gRelays[channel]);
      } else {
        Serial.printf("Ignoring command for channel %d (board has %d)\n",
                      channel, CHANNEL_COUNT);
      }
      return;
    }

    case CMD_SET:
      if (len < 3) {
        Serial.println("Ignoring set without a channel and a state byte");
        return;
      }
      Serial.printf("Command: channel %d %s\n", packet[1],
                    packet[2] ? "on" : "off");
      applyRelay(packet[1], packet[2] != 0);
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

// Client must write the shared API key here before a relay can be switched.
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
      notifySensorJson(dataChar, relayJson());
    }
  }
};

// Serve the current board state only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? relayJson().c_str() : "{}");
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

  // Accept both write flavours, like the LED node: a client that can use
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

void transportPublish() {
  if (!deviceConnected || !authed) return;
  String out = relayJson();
  notifySensorJson(dataChar, out);
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Playground Relay Node ---");

  if (CHANNEL_COUNT > MAX_CHANNELS) {
    Serial.println("More channels configured than this node supports");
  }

#if RELAY_INTERFACE == RELAY_IFACE_GPIO
  for (int channel = 0; channel < CHANNEL_COUNT; channel++) {
    pinMode(RELAY_PINS[channel], OUTPUT);
  }
#else
  Wire.begin();
#endif

  // Start with every contact open, driving the board explicitly so the
  // reported state and the actual coils agree from the first moment. Not
  // applyAllRelays(): no transport is up yet to publish through.
  for (int channel = 0; channel < CHANNEL_COUNT; channel++) {
    gRelays[channel] = false;
  }
  writeRelayBoard();

  transportSetup();

  Serial.printf("Actuator: %s (%d channel%s)\n", SENSOR_NAME, CHANNEL_COUNT,
                CHANNEL_COUNT == 1 ? "" : "s");
}

// ============================================================
void loop() {
  transportLoop();
  delay(1);
}
