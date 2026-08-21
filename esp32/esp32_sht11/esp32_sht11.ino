/*
 * Sensor Playground Sensor Node — ESP32 + SHT11 (Sensirion SHT1x)
 *
 * Implements the Sensor Playground Sensor Interface (see docs/sensor.md).
 * Reads a Sensirion SHT1x (temperature, humidity) — the classic SHT10 /
 * SHT11 / SHT15 family. The chips differ only in calibration accuracy and
 * speak the same proprietary two-wire protocol (SCK + bidirectional DATA);
 * it resembles I2C but is NOT I2C — the sensor cannot share an I2C bus.
 * Set SENSOR_NAME to the chip on your board so the app shows the right
 * name; nothing else changes.
 *
 * The protocol is implemented right here (no library): the bus is fully
 * master-clocked with no minimum speed, so plain digitalWrite bit-banging
 * is reliable. The DATA line is driven open-drain style — released via
 * INPUT_PULLUP for a 1, pulled LOW for a 0 — because the sensor drives the
 * same wire when answering.
 *
 * The sensor must not be measured more than ~10% of the time or it heats
 * itself; readings are taken at most every two seconds and cached, and a
 * failed read serves the previous value for up to 30 seconds before the
 * value keys are omitted (which the app treats as "no fresh data").
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — UDP discovery (9133) + HTTPS REST (9132), self-signed cert
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then reads/subscribes
 *                    DATA_CHAR_UUID for the same JSON payload.
 *
 * The JSON payload is identical on both transports, so the app parses it the
 * same way regardless of how it arrived.
 *
 * Required Libraries:
 *   ArduinoJson, and for TRANSPORT_WIFI: WiFi, WiFiUdp.
 *   (BLE uses the ESP32 core's built-in BLE stack; the sensor needs none.)
 */

// ============================================================
//  HARDWARE CONFIGURATION
// ============================================================
// GPIOs of the SHT1x two-wire bus.
const int SHT_DATA_PIN = 4;
const int SHT_SCK_PIN  = 5;

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
  #include "mbedtls/ssl.h"
  #include "mbedtls/pk.h"
  #include "mbedtls/x509_crt.h"
  #include "mbedtls/entropy.h"
  #include "mbedtls/ctr_drbg.h"
  #include "mbedtls/net_sockets.h"
  #include "mbedtls/error.h"
  #include "../common/sensor_tls_runtime.h"
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
#endif

// --- Sensor ---
// Set to "SHT10" or "SHT15" if that is the chip on your board.
const char* SENSOR_NAME = "SHT11";

String buildSensorJson(bool shortKeys);

// ============================================================
//  SHT1x PROTOCOL (datasheet V5; commands 000 00011 / 000 00101)
// ============================================================
const uint8_t SHT1X_CMD_TEMPERATURE = 0x03;
const uint8_t SHT1X_CMD_HUMIDITY    = 0x05;

// 14-bit temperature at 3.3V supply: T = D1 + 0.01 * raw.
const float SHT1X_D1 = -39.66f;

// 12-bit humidity polynomial + temperature compensation (datasheet V4/V5).
const float SHT1X_C1 = -2.0468f;
const float SHT1X_C2 = 0.0367f;
const float SHT1X_C3 = -1.5955e-6f;
const float SHT1X_T1 = 0.01f;
const float SHT1X_T2 = 0.00008f;

// A 14-bit measurement takes up to 320 ms.
const unsigned long SHT1X_MEASURE_TIMEOUT_MS = 400;

// Self-heating limit: read the sensor at most every two seconds, and serve
// the cached values for up to 30 seconds when reads fail.
const unsigned long READ_INTERVAL_MS = 2000;
const unsigned long CACHE_MAX_AGE_MS = 30000;

float gTemperature = 0.0f;
float gHumidity = 0.0f;
unsigned long gCachedAtMs = 0;
unsigned long gAttemptedAtMs = 0;
bool gHaveReading = false;

// The DATA line is bidirectional: release it (a pull-up makes it HIGH, and
// lets the sensor answer) or actively pull it LOW. Never drive it HIGH —
// the sensor may be driving LOW at the same moment.
void shtDataRelease() { pinMode(SHT_DATA_PIN, INPUT_PULLUP); }

void shtDataLow() {
  pinMode(SHT_DATA_PIN, OUTPUT);
  digitalWrite(SHT_DATA_PIN, LOW);
}

void shtSckPulse() {
  digitalWrite(SHT_SCK_PIN, HIGH);
  delayMicroseconds(2);
  digitalWrite(SHT_SCK_PIN, LOW);
  delayMicroseconds(2);
}

// "Transmission start": DATA falls while SCK is high, then rises while SCK
// is high again — a pattern that cannot occur inside a byte transfer.
void shtTransmissionStart() {
  shtDataRelease();
  digitalWrite(SHT_SCK_PIN, LOW);
  delayMicroseconds(2);
  digitalWrite(SHT_SCK_PIN, HIGH);
  delayMicroseconds(2);
  shtDataLow();
  delayMicroseconds(2);
  digitalWrite(SHT_SCK_PIN, LOW);
  delayMicroseconds(2);
  digitalWrite(SHT_SCK_PIN, HIGH);
  delayMicroseconds(2);
  shtDataRelease();
  delayMicroseconds(2);
  digitalWrite(SHT_SCK_PIN, LOW);
  delayMicroseconds(2);
}

// If the master and sensor ever fall out of step: DATA high while toggling
// SCK nine or more times resynchronises the interface.
void shtConnectionReset() {
  shtDataRelease();
  digitalWrite(SHT_SCK_PIN, LOW);
  for (int i = 0; i < 10; i++) shtSckPulse();
}

// Sends one command byte; returns false when the sensor does not ACK.
bool shtSendCommand(uint8_t command) {
  shtTransmissionStart();
  for (int bit = 7; bit >= 0; bit--) {
    if (command & (1 << bit)) {
      shtDataRelease();
    } else {
      shtDataLow();
    }
    delayMicroseconds(2);
    shtSckPulse();
  }
  // ACK: the sensor pulls DATA low during the ninth clock.
  shtDataRelease();
  delayMicroseconds(2);
  digitalWrite(SHT_SCK_PIN, HIGH);
  delayMicroseconds(2);
  bool acked = digitalRead(SHT_DATA_PIN) == LOW;
  digitalWrite(SHT_SCK_PIN, LOW);
  delayMicroseconds(2);
  return acked;
}

// Reads one byte; [ack] low keeps the transfer going, high ends it (the
// sensor then skips the CRC byte, which this driver does not use).
uint8_t shtReadByte(bool ack) {
  uint8_t value = 0;
  shtDataRelease();
  for (int bit = 7; bit >= 0; bit--) {
    digitalWrite(SHT_SCK_PIN, HIGH);
    delayMicroseconds(2);
    if (digitalRead(SHT_DATA_PIN) == HIGH) value |= (1 << bit);
    digitalWrite(SHT_SCK_PIN, LOW);
    delayMicroseconds(2);
  }
  if (ack) {
    shtDataLow();
  } else {
    shtDataRelease();
  }
  delayMicroseconds(2);
  shtSckPulse();
  shtDataRelease();
  return value;
}

// Runs one measurement command and returns the raw 14/12-bit result, or -1.
int shtMeasure(uint8_t command) {
  if (!shtSendCommand(command)) return -1;

  // The sensor releases DATA while measuring and pulls it low when done.
  unsigned long start = millis();
  while (digitalRead(SHT_DATA_PIN) == HIGH) {
    if (millis() - start > SHT1X_MEASURE_TIMEOUT_MS) return -1;
    delay(5);
  }

  int msb = shtReadByte(true);
  int lsb = shtReadByte(false);
  return (msb << 8) | lsb;
}

// Converts raw readings per the datasheet (3.3V supply, 14/12-bit).
float shtTemperature(int rawTemperature) {
  return SHT1X_D1 + SHT1X_T1 * rawTemperature;
}

float shtHumidity(int rawHumidity, float temperature) {
  float linear = SHT1X_C1 + SHT1X_C2 * rawHumidity +
                 SHT1X_C3 * (float)rawHumidity * (float)rawHumidity;
  float compensated =
      (temperature - 25.0f) * (SHT1X_T1 + SHT1X_T2 * rawHumidity) + linear;
  return constrain(compensated, 0.0f, 100.0f);
}

// Refreshes the cached reading, at most every READ_INTERVAL_MS. A failed
// read keeps the previous values; the cache only expires after
// CACHE_MAX_AGE_MS without a successful read.
void refreshReading() {
  unsigned long now = millis();
  if (gHaveReading && now - gAttemptedAtMs < READ_INTERVAL_MS) return;
  gAttemptedAtMs = now;

  int rawTemperature = shtMeasure(SHT1X_CMD_TEMPERATURE);
  int rawHumidity =
      rawTemperature >= 0 ? shtMeasure(SHT1X_CMD_HUMIDITY) : -1;
  if (rawTemperature < 0 || rawHumidity < 0) {
    Serial.println("SHT1x read failed (serving cache)");
    shtConnectionReset();
    if (gHaveReading && now - gCachedAtMs > CACHE_MAX_AGE_MS) {
      gHaveReading = false;
    }
    return;
  }

  gTemperature = shtTemperature(rawTemperature);
  gHumidity = shtHumidity(rawHumidity, gTemperature);
  gCachedAtMs = now;
  gHaveReading = true;
}

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
SemaphoreHandle_t sensorReadMutex = nullptr;
TaskHandle_t tlsServerTaskHandle = nullptr;
#endif

void lockSensorStateForTransport() {
#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
  xSemaphoreTake(sensorReadMutex, portMAX_DELAY);
#endif
}

void unlockSensorStateForTransport() {
#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
  xSemaphoreGive(sensorReadMutex);
#endif
}

String buildSensorJsonForTransport(bool shortKeys) {
  lockSensorStateForTransport();
  String json = buildSensorJson(shortKeys);
  unlockSensorStateForTransport();
  return json;
}

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
// ============================================================
//  WIFI TRANSPORT
// ============================================================
const int HTTPS_PORT = 9132;
const int UDP_PORT   = 9133;

WiFiUDP udp;
WiFiServer tcpServer(HTTPS_PORT);

mbedtls_ssl_config sslConf;
mbedtls_x509_crt srvcert;
mbedtls_pk_context pkey;
mbedtls_entropy_context entropy;
mbedtls_ctr_drbg_context ctr_drbg;

void handleUdpDiscovery();
void handleTlsClient();
void tlsServerTask(void* parameter);

void transportSetup() {
  if (!connectSensorWifi(WIFI_SSID, WIFI_PASS)) {
    Serial.println("Restarting after WiFi setup failure");
    delay(1000);
    ESP.restart();
  }

  mbedtls_ssl_config_init(&sslConf);
  mbedtls_x509_crt_init(&srvcert);
  mbedtls_pk_init(&pkey);
  mbedtls_entropy_init(&entropy);
  mbedtls_ctr_drbg_init(&ctr_drbg);

  if (!configureSensorTls(&sslConf, &srvcert, &pkey, &entropy, &ctr_drbg,
                          SERVER_CERT, SERVER_KEY)) {
    Serial.println("TLS initialization failed; restarting");
    delay(1000);
    ESP.restart();
    return;
  }

  sensorReadMutex = xSemaphoreCreateMutex();
  if (sensorReadMutex == nullptr) {
    Serial.println("Sensor mutex allocation failed; restarting");
    delay(1000);
    ESP.restart();
    return;
  }

  tcpServer.begin();
  udp.begin(UDP_PORT);
  if (xTaskCreate(tlsServerTask, "sensor-tls", 12288, nullptr, 1,
                  &tlsServerTaskHandle) != pdPASS) {
    Serial.println("TLS task creation failed; restarting");
    delay(1000);
    ESP.restart();
    return;
  }
  Serial.println("HTTPS on port 9132, UDP on port 9133");
}

void transportLoop() {
  if (!sensorWifiReady()) {
    delay(10);
    return;
  }
  handleUdpDiscovery();
  delay(1);
}

void handleUdpDiscovery() {
  int packetSize = udp.parsePacket();
  if (!packetSize) return;

  char buffer[64];
  int len = udp.read(buffer, sizeof(buffer) - 1);
  buffer[len] = '\0';
  if (strstr(buffer, "SENSOR_TESTER") == NULL) return;

  String json = buildSensorJsonForTransport(true);
  udp.beginPacket(udp.remoteIP(), udp.remotePort());
  udp.print(json);
  udp.endPacket();
}

static int tlsSend(void* ctx, const unsigned char* buf, size_t len) {
  return ((WiFiClient*)ctx)->write(buf, len);
}

static int tlsRecv(void* ctx, unsigned char* buf, size_t len) {
  WiFiClient* client = (WiFiClient*)ctx;
  unsigned long start = millis();
  while (!client->available() && millis() - start < 200) delay(1);
  if (!client->available()) return MBEDTLS_ERR_SSL_WANT_READ;
  return client->read(buf, len);
}

void tlsServerTask(void* parameter) {
  (void)parameter;
  for (;;) {
    if (WiFi.status() == WL_CONNECTED) handleTlsClient();
    vTaskDelay(pdMS_TO_TICKS(1));
  }
}

void handleTlsClient() {
  WiFiClient client = tcpServer.accept();
  if (!client) return;

  mbedtls_ssl_context ssl;
  mbedtls_ssl_init(&ssl);
  if (mbedtls_ssl_setup(&ssl, &sslConf) != 0) {
    mbedtls_ssl_free(&ssl);
    client.stop();
    return;
  }
  mbedtls_ssl_set_bio(&ssl, &client, tlsSend, tlsRecv, NULL);

  // Overall deadline for one client interaction. The TLS server has its own
  // task, so a stalled client cannot block discovery or the sensor loop.
  const unsigned long tlsStart = millis();
  int ret;
  do {
    ret = mbedtls_ssl_handshake(&ssl);
  } while ((ret == MBEDTLS_ERR_SSL_WANT_READ || ret == MBEDTLS_ERR_SSL_WANT_WRITE) &&
           millis() - tlsStart < 4000);

  if (ret != 0) {
    mbedtls_ssl_free(&ssl);
    client.stop();
    return;
  }

  char reqBuf[1024];
  const int reqLen = readSensorHttpRequest(
      &ssl, reqBuf, sizeof(reqBuf), tlsStart, 4000);
  if (reqLen <= 0) {
    mbedtls_ssl_free(&ssl);
    client.stop();
    return;
  }

  String headers(reqBuf);
  String apiKey = "";
  int keyIdx = headers.indexOf("X-Api-Key:");
  if (keyIdx == -1) keyIdx = headers.indexOf("x-api-key:");
  if (keyIdx >= 0) {
    int valStart = keyIdx + 10;
    int valEnd = headers.indexOf('\n', valStart);
    if (valEnd == -1) valEnd = headers.length();
    apiKey = headers.substring(valStart, valEnd);
    apiKey.trim();
  }

  String response;
  const bool validTarget =
      headers.startsWith("GET / HTTP/1.1\r\n") ||
      headers.startsWith("GET / HTTP/1.0\r\n");
  if (!validTarget) {
    response = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n"
               "Connection: close\r\n\r\n";
  } else if (apiKey != String(API_KEY)) {
    response = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n"
               "Connection: close\r\n\r\n";
  } else {
    String json = buildSensorJsonForTransport(false);
    response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n"
               "Content-Length: " + String(json.length()) +
               "\r\nConnection: close\r\n\r\n" + json;
  }

  if (!writeSensorTlsResponse(&ssl, response, tlsStart, 4000)) {
    Serial.println("TLS response write failed");
  }
  mbedtls_ssl_close_notify(&ssl);
  mbedtls_ssl_free(&ssl);
  client.stop();
}

#else
// ============================================================
//  BLE TRANSPORT
// ============================================================
BLECharacteristic* dataChar = nullptr;
bool deviceConnected = false;
bool authed = false;
unsigned long lastNotifyMs = 0;

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
  }
};

// Serve the latest reading only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    characteristic->setValue(authed ? buildSensorJsonForTransport(false).c_str() : "{}");
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
  // Push a fresh reading once per second to subscribed, authorized clients.
  if (deviceConnected && authed && millis() - lastNotifyMs >= 1000) {
    lastNotifyMs = millis();
    String json = buildSensorJsonForTransport(false);
    if (json.indexOf("temperature") >= 0) {
      notifySensorJson(dataChar, json);
    }
  }
  delay(10);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Playground Sensor Node ---");

  pinMode(SHT_SCK_PIN, OUTPUT);
  digitalWrite(SHT_SCK_PIN, LOW);
  shtDataRelease();
  // The sensor needs 11 ms after power-up before the first command.
  delay(20);
  shtConnectionReset();

  refreshReading();
  if (!gHaveReading) {
    Serial.printf("Warning: no reading from the %s (DATA=%d, SCK=%d) yet\n",
                  SENSOR_NAME, SHT_DATA_PIN, SHT_SCK_PIN);
  }

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  transportLoop();
}

// ============================================================
// Build sensor JSON (identical payload on every transport).
// ============================================================
String buildSensorJson(bool shortKeys) {
  StaticJsonDocument<512> doc;

  if (shortKeys) {
    doc["type"] = SENSOR_NAME;
    doc["host"] = HOSTNAME;
#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
    doc["ip"]   = WiFi.localIP().toString();
    doc["port"] = HTTPS_PORT;
#endif
  } else {
    doc["sensor"] = SENSOR_NAME;
    doc["host"]   = HOSTNAME;
  }

  refreshReading();
  if (gHaveReading) {
    if (shortKeys) {
      doc["temp"] = gTemperature;
      doc["hum"]  = gHumidity;
    } else {
      doc["temperature"] = gTemperature;
      doc["humidity"]    = gHumidity;
    }
  }

  String output;
  serializeJson(doc, output);
  return output;
}
