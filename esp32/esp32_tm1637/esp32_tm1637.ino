/*
 * Sensor Tester Clock Node — ESP32 + Grove 4-Digit Display (TM1637)
 *
 * Implements the *actuator* variant of the Sensor Tester Sensor Interface.
 * Like the LED node it talks in both directions: the app pushes the time to
 * show (and a brightness), and the node reports the state it is actually
 * displaying — which keeps changing on its own, because once a time is set
 * the node advances the minute and blinks the colon autonomously. The node
 * therefore owns the state and the app only ever renders what the node last
 * reported.
 *
 * Over Wi-Fi both directions are JSON messages on the WebSocket:
 *
 *     app -> node   {"time":"HH:MM"}    set the displayed time (24-hour)
 *                   {"brightness":0..7} set the brightness (clamped)
 *     node -> app   {"time":"HH:MM","brightness":3}   current state
 *                   {"time":null,"brightness":3}      no time set yet
 *
 * The node pushes its state on connect, after every accepted command, and on
 * each minute rollover — never on the colon blink, so state traffic stays at
 * one message a minute. Before the first time set the display shows "--:--".
 *
 * Over BLE the state is a notify on the data characteristic carrying the
 * same JSON, and commands are short binary writes to the command
 * characteristic:
 *
 *     0x01 <hh> <mm>   set the time (rejected unless hh<=23 and mm<=59)
 *     0x02 <0..7>      set the brightness (clamped)
 *     0x03             re-notify the current state
 *
 * The TM1637 is bit-banged directly (two GPIOs, a proprietary two-wire
 * protocol similar to I2C but without addresses) — no display library
 * needed, matching the SSD1306 node's raw-Wire approach.
 * See ../../python/tm1637/sensor_node.py for the same logic in Python.
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
// GPIOs the Grove display's two-wire bus is connected to (Grove yellow wire
// = CLK, white wire = DIO).
const int TM_CLK_PIN = 5;
const int TM_DIO_PIN = 4;

// Brightness the display starts with (0 dimmest .. 7 brightest; 0 is still
// lit — the TM1637 has no off level below it).
const uint8_t DEFAULT_BRIGHTNESS = 3;

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

const char* SENSOR_NAME = "TM1637";

// Command opcodes (see the header comment).
const uint8_t CMD_SET_TIME      = 0x01;
const uint8_t CMD_BRIGHTNESS    = 0x02;
const uint8_t CMD_STATE_REQUEST = 0x03;

// Colon blink half-period: on for 500 ms, off for 500 ms, like most clocks.
const unsigned long COLON_BLINK_MS = 500;

// Displayed time, the single source of truth this node publishes. -1 means
// no time has been set yet and the display shows "--:--".
int gHour = -1;
int gMinute = -1;
uint8_t gBrightness = DEFAULT_BRIGHTNESS;

// Colon phase and the local timekeeping accumulators.
bool gColonOn = false;
unsigned long gLastBlinkMs = 0;
unsigned long gLastTickMs = 0;
unsigned long gMinuteAccumMs = 0;

void transportPublish();

// ============================================================
//  TM1637 BIT-BANGED DRIVER
// ============================================================
// The chip speaks a two-wire protocol of its own: start/stop conditions like
// I2C, LSB-first bytes, and an ACK slot after every byte. It has no minimum
// clock speed, and tops out around 250 kHz — a 3 us half-clock keeps well
// inside that.
const unsigned int TM_HALF_CLOCK_US = 3;

// Segment patterns for the digits 0-9, gfedcba bit order.
const uint8_t SEGMENT_DIGITS[10] = {
  0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F
};
// A lone middle segment (g), the "no time yet" placeholder digit.
const uint8_t SEGMENT_DASH = 0x40;
// The Grove module wires the colon to bit 7 of the second digit.
const uint8_t SEGMENT_COLON = 0x80;

void tmDelay() { delayMicroseconds(TM_HALF_CLOCK_US); }

void tmStart() {
  // DIO falls while CLK is high.
  digitalWrite(TM_CLK_PIN, HIGH);
  digitalWrite(TM_DIO_PIN, HIGH);
  tmDelay();
  digitalWrite(TM_DIO_PIN, LOW);
  tmDelay();
  digitalWrite(TM_CLK_PIN, LOW);
  tmDelay();
}

void tmStop() {
  // DIO rises while CLK is high.
  digitalWrite(TM_CLK_PIN, LOW);
  digitalWrite(TM_DIO_PIN, LOW);
  tmDelay();
  digitalWrite(TM_CLK_PIN, HIGH);
  tmDelay();
  digitalWrite(TM_DIO_PIN, HIGH);
  tmDelay();
}

void tmWriteByte(uint8_t value) {
  for (int bit = 0; bit < 8; bit++) {
    digitalWrite(TM_CLK_PIN, LOW);
    digitalWrite(TM_DIO_PIN, (value >> bit) & 1 ? HIGH : LOW);
    tmDelay();
    digitalWrite(TM_CLK_PIN, HIGH);
    tmDelay();
  }
  // ACK slot: release DIO for one clock; the chip pulls it low. The level is
  // not checked — there is nothing useful to do on a NAK, and the next state
  // push tells the app what the display actually shows.
  digitalWrite(TM_CLK_PIN, LOW);
  pinMode(TM_DIO_PIN, INPUT_PULLUP);
  tmDelay();
  digitalWrite(TM_CLK_PIN, HIGH);
  tmDelay();
  digitalWrite(TM_CLK_PIN, LOW);
  pinMode(TM_DIO_PIN, OUTPUT);
  digitalWrite(TM_DIO_PIN, LOW);
}

// Write all four digits and the brightness in one transaction burst.
void tmRender(const uint8_t segments[4], uint8_t brightness) {
  tmStart();
  tmWriteByte(0x40);  // data command: write, auto-increment address
  tmStop();

  tmStart();
  tmWriteByte(0xC0);  // address command: start at digit 0
  for (int i = 0; i < 4; i++) {
    tmWriteByte(segments[i]);
  }
  tmStop();

  tmStart();
  tmWriteByte(0x88 | (brightness & 0x07));  // display on at this brightness
  tmStop();
}

// Render the current clock state (time or "--:--", colon phase, brightness).
void renderClock() {
  uint8_t segments[4];
  if (gHour < 0 || gMinute < 0) {
    for (int i = 0; i < 4; i++) segments[i] = SEGMENT_DASH;
  } else {
    segments[0] = SEGMENT_DIGITS[gHour / 10];
    segments[1] = SEGMENT_DIGITS[gHour % 10];
    segments[2] = SEGMENT_DIGITS[gMinute / 10];
    segments[3] = SEGMENT_DIGITS[gMinute % 10];
    if (gColonOn) segments[1] |= SEGMENT_COLON;
  }
  tmRender(segments, gBrightness);
}

// ============================================================
//  TRANSPORT-AGNOSTIC CLOCK STATE
// ============================================================

// Returns the state payload ({"time":"12:34","brightness":3}; time is null
// while no time has been set yet).
String clockJson() {
  StaticJsonDocument<64> doc;
  if (gHour < 0 || gMinute < 0) {
    doc["time"] = nullptr;
  } else {
    char timeText[6];
    snprintf(timeText, sizeof(timeText), "%02d:%02d", gHour, gMinute);
    doc["time"] = timeText;
  }
  doc["brightness"] = gBrightness;
  String out;
  serializeJson(doc, out);
  return out;
}

// Set the displayed time and publish the new state. Publishing
// unconditionally (rather than only on a change) keeps a redundant command
// from leaving a client that guessed wrong without a correction.
void applyTime(int hour, int minute) {
  gHour = hour;
  gMinute = minute;
  // Restart the minute and blink phase so ":00 seconds" is now, and give
  // immediate feedback with a lit colon.
  gMinuteAccumMs = 0;
  gColonOn = true;
  gLastBlinkMs = millis();
  renderClock();
  transportPublish();
}

void applyBrightness(int brightness) {
  if (brightness < 0) brightness = 0;
  if (brightness > 7) brightness = 7;
  gBrightness = (uint8_t)brightness;
  renderClock();
  transportPublish();
}

// Advance the local clock and blink the colon. Only the minute rollover
// publishes; the blink is render-only.
void tickClock() {
  unsigned long now = millis();
  unsigned long elapsed = now - gLastTickMs;
  gLastTickMs = now;

  if (gHour < 0 || gMinute < 0) return;

  if (now - gLastBlinkMs >= COLON_BLINK_MS) {
    gLastBlinkMs = now;
    gColonOn = !gColonOn;
    renderClock();
  }

  gMinuteAccumMs += elapsed;
  if (gMinuteAccumMs < 60000UL) return;
  gMinuteAccumMs -= 60000UL;
  if (++gMinute >= 60) {
    gMinute = 0;
    if (++gHour >= 24) gHour = 0;  // midnight rollover
  }
  renderClock();
  transportPublish();
}

// Parse and apply an "HH:MM" time string; returns false when malformed.
bool applyTimeText(const char* text) {
  if (text == nullptr || strlen(text) != 5 || text[2] != ':') return false;
  for (int i : {0, 1, 3, 4}) {
    if (text[i] < '0' || text[i] > '9') return false;
  }
  int hour = (text[0] - '0') * 10 + (text[1] - '0');
  int minute = (text[3] - '0') * 10 + (text[4] - '0');
  if (hour > 23 || minute > 59) return false;
  applyTime(hour, minute);
  return true;
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

  if (doc["time"].is<const char*>()) {
    const char* timeText = doc["time"];
    if (applyTimeText(timeText)) {
      Serial.printf("Command: time %s\n", timeText);
    } else {
      Serial.println("Ignoring invalid time");
    }
    return;
  }

  if (doc["brightness"].is<int>()) {
    int brightness = doc["brightness"];
    Serial.printf("Command: brightness %d\n", brightness);
    applyBrightness(brightness);
    return;
  }

  Serial.println("Ignoring unknown command");
}

void webSocketEvent(uint8_t num, WStype_t type, uint8_t* payload, size_t len) {
  switch (type) {
    case WStype_CONNECTED: {
      Serial.printf("[%u] Client connected\n", num);
      // Send the current state so the app shows the display as it actually
      // is rather than waiting for the next minute. sendTXT takes String&
      // (an lvalue), so hold it in a variable.
      String out = clockJson();
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

  // A clock node has no readings to advertise, only its identity.
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
  String out = clockJson();
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
    case CMD_SET_TIME:
      if (len < 3 || packet[1] > 23 || packet[2] > 59) {
        Serial.println("Ignoring invalid time command");
        return;
      }
      Serial.printf("Command: time %02d:%02d\n", packet[1], packet[2]);
      applyTime(packet[1], packet[2]);
      return;

    case CMD_BRIGHTNESS:
      if (len < 2) {
        Serial.println("Ignoring brightness without a value byte");
        return;
      }
      Serial.printf("Command: brightness %d\n", packet[1]);
      applyBrightness(packet[1]);
      return;

    case CMD_STATE_REQUEST:
      Serial.println("Command: state request");
      transportPublish();
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

// Client must write the shared API key here before the clock can be set.
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
      notifySensorJson(dataChar, clockJson());
    }
  }
};

// Serve the current clock state only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? clockJson().c_str() : "{}");
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
  // write-without-response gets a slightly snappier command.
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
  String out = clockJson();
  notifySensorJson(dataChar, out);
  Serial.println(out);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Clock Node ---");

  pinMode(TM_CLK_PIN, OUTPUT);
  pinMode(TM_DIO_PIN, OUTPUT);
  digitalWrite(TM_CLK_PIN, LOW);
  digitalWrite(TM_DIO_PIN, LOW);

  // Show "--:--" until the app sets a time, driving the panel directly. Not
  // applyTime(): no transport is up yet to publish through, and there is no
  // time to publish anyway.
  gLastTickMs = millis();
  gLastBlinkMs = gLastTickMs;
  renderClock();

  transportSetup();

  Serial.print("Actuator: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();
  tickClock();
  delay(1);
}
