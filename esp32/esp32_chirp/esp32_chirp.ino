/*
 * Sensor Playground Sensor Node — ESP32 + Chirp I2C Soil Moisture Sensor
 *
 * Implements the Sensor Playground Sensor Interface (see docs/sensor.md).
 * Reads a Catnip Electronics I2C Soil Moisture Sensor ("Chirp", default
 * address 0x20) over I2C: soil capacitance (mapped onto a moisture
 * percentage between the two calibration points below), soil temperature,
 * and the ambient light level.
 * https://github.com/Apollon77/I2CSoilMoistureSensor
 *
 * The chip measures light by timing a phototransistor discharge, which takes
 * up to three seconds — so the light measurement runs as a non-blocking
 * state machine in the loop, and the payload carries the last completed one
 * (inverted to raw brightness counts, so higher = brighter like the Grove
 * light node). Until the first measurement completes the `light` key is
 * simply absent.
 *
 * Unlike the digital contact sensors this is a *pollable* node (continuous
 * values), so it uses the same HTTPS REST + UDP discovery transport as the
 * environment sensors rather than the WebSocket push path.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — UDP discovery (9133) + HTTPS REST (9132), self-signed cert
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then reads/subscribes
 *                    DATA_CHAR_UUID for the same JSON payload.
 *
 * Required Libraries:
 *   ArduinoJson, I2CSoilMoistureSensor, and for TRANSPORT_WIFI: WiFi, WiFiUdp.
 */

// --- Transport selection (change this line) ---
#define TRANSPORT_WIFI 0
#define TRANSPORT_BLE  1
#ifndef ACTIVE_TRANSPORT
#define ACTIVE_TRANSPORT TRANSPORT_BLE
#endif

#include <ArduinoJson.h>
#include <Wire.h>
#include <I2CSoilMoistureSensor.h>
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
const char* SENSOR_NAME = "CHIRP";

// Default I2C address of the sensor; change it with the library's
// setAddress() if you re-addressed yours.
I2CSoilMoistureSensor soil;  // 0x20

// Calibration: the raw capacitance of *your* sensor in dry air and fully
// submerged in water (capacitance rises with moisture, so wet > dry). Read
// the `cap` value from the REST payload in both situations and put the
// numbers here — individual sensors vary.
const int CAP_DRY = 290;
const int CAP_WET = 520;

// How often capacitance and temperature are re-read.
const unsigned long READ_INTERVAL_MS = 1000;

// A light measurement takes up to three seconds on the chip.
const unsigned long LIGHT_MEASURE_MS = 3000;

// Latest readings, cached by updateSensor() in the loop so the transport
// callbacks never touch the I2C bus themselves.
unsigned int gCapacitance = 0;
float gTemperature = 0.0f;
long gLight = -1;  // inverted brightness; -1 until the first measurement
bool gLightPending = false;
unsigned long gLightStartMs = 0;
unsigned long gLastReadMs = 0;

String buildSensorJson(bool shortKeys);

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

// Maps a raw capacitance onto 0-100 % between the calibration points.
int moisturePercent(unsigned int capacitance) {
  long percent = 100L * ((long)capacitance - CAP_DRY) / (CAP_WET - CAP_DRY);
  if (percent < 0) percent = 0;
  if (percent > 100) percent = 100;
  return (int)percent;
}

// Refreshes the cached readings; light runs as a non-blocking measurement.
void updateSensor() {
  unsigned long now = millis();

  if (now - gLastReadMs >= READ_INTERVAL_MS) {
    gLastReadMs = now;
    unsigned int capacitance = soil.getCapacitance();
    float temperature = soil.getTemperature() / 10.0f;
    lockSensorStateForTransport();
    gCapacitance = capacitance;
    gTemperature = temperature;
    unlockSensorStateForTransport();
  }

  if (!gLightPending) {
    soil.startMeasureLight();
    gLightPending = true;
    gLightStartMs = now;
  } else if (now - gLightStartMs >= LIGHT_MEASURE_MS) {
    unsigned int raw = soil.getLight(false);
    gLightPending = false;
    lockSensorStateForTransport();
    gLight = 65535L - raw;  // the chip counts up in darkness; invert
    unlockSensorStateForTransport();
  }
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
    notifySensorJson(dataChar, buildSensorJsonForTransport(false));
  }
  delay(10);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Playground Sensor Node ---");

  Wire.begin();
  soil.begin(true);  // resets the sensor and waits for it to come up
  Serial.print("Chirp sensor at 0x");
  Serial.print(soil.getAddress(), HEX);
  Serial.print(", firmware version 0x");
  Serial.println(soil.getVersion(), HEX);

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  updateSensor();
  transportLoop();
}

// ============================================================
// Build sensor JSON (identical quantities on every transport).
// ============================================================
String buildSensorJson(bool shortKeys) {
  StaticJsonDocument<256> doc;

  if (shortKeys) {
    doc["type"] = SENSOR_NAME;
    doc["host"] = HOSTNAME;
#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
    doc["ip"]   = WiFi.localIP().toString();
    doc["port"] = HTTPS_PORT;
#endif
    doc["moist"] = moisturePercent(gCapacitance);
    doc["temp"]  = gTemperature;
    if (gLight >= 0) doc["light"] = gLight;
  } else {
    doc["sensor"] = SENSOR_NAME;
    doc["host"]   = HOSTNAME;
    doc["moisture"]    = moisturePercent(gCapacitance);
    doc["temperature"] = gTemperature;
    if (gLight >= 0) doc["light"] = gLight;
    // The raw capacitance, for calibrating CAP_DRY/CAP_WET.
    doc["cap"] = gCapacitance;
  }

  String output;
  serializeJson(doc, output);
  return output;
}
