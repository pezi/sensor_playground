/*
 * Sensor Playground Sensor Node — ESP32 + BME680
 *
 * Implements the Sensor Playground Sensor Interface (see docs/sensor.md).
 * Reads a BME680 (temperature, humidity, pressure, IAQ) via I2C.
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
 *   ArduinoJson, Wire, Adafruit BME680 Library,
 *   and for TRANSPORT_WIFI: WiFi, WiFiUdp.
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
#include <Adafruit_BME680.h>
#include "bme680_logic.h"
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
Adafruit_BME680 bme;
const char* SENSOR_NAME = "BME680";
const uint8_t BME680_I2C_ADDRESSES[] = {0x76, 0x77};
// FIELD0 starts at 0x1d; byte 14 carries gas-valid and heater-stable.
const uint8_t BME680_GAS_STATUS_REGISTER = 0x2B;
uint8_t bme680I2cAddress = 0;

// --- IAQ Calculation (ported from dart_periphery BME680 driver) ---
Bme680Iaq airQuality;

String buildSensorJson(bool shortKeys);
bool beginBme680();
bool readBme680GasStatus(uint8_t& status);

SemaphoreHandle_t sensorReadMutex = nullptr;
#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
TaskHandle_t tlsServerTaskHandle = nullptr;
#endif

void lockSensorStateForTransport() {
  xSemaphoreTake(sensorReadMutex, portMAX_DELAY);
}

void unlockSensorStateForTransport() {
  xSemaphoreGive(sensorReadMutex);
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
SemaphoreHandle_t bleStateMutex = nullptr;
String latestSensorJson = "{}";

bool bleClientReady() {
  xSemaphoreTake(bleStateMutex, portMAX_DELAY);
  const bool ready = deviceConnected && authed;
  xSemaphoreGive(bleStateMutex);
  return ready;
}

class ServerCallbacks : public BLEServerCallbacks {
  void onConnect(BLEServer* server) override {
    xSemaphoreTake(bleStateMutex, portMAX_DELAY);
    deviceConnected = true;
    xSemaphoreGive(bleStateMutex);
  }
  void onDisconnect(BLEServer* server) override {
    xSemaphoreTake(bleStateMutex, portMAX_DELAY);
    deviceConnected = false;
    authed = false;
    xSemaphoreGive(bleStateMutex);
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
    xSemaphoreTake(bleStateMutex, portMAX_DELAY);
    authed = (val == API_KEY);
    const bool accepted = authed;
    xSemaphoreGive(bleStateMutex);
    Serial.println(accepted ? "Client authorized" : "Bad API key");
  }
};

// Serve the latest reading only to an authorized client.
class DataCallbacks : public BLECharacteristicCallbacks {
  void onRead(BLECharacteristic* characteristic) override {
    xSemaphoreTake(bleStateMutex, portMAX_DELAY);
    characteristic->setValue(authed ? latestSensorJson.c_str() : "{}");
    xSemaphoreGive(bleStateMutex);
  }
};

void transportSetup() {
  bleStateMutex = xSemaphoreCreateMutex();
  if (bleStateMutex == nullptr) {
    Serial.println("BLE state mutex allocation failed; restarting");
    delay(1000);
    ESP.restart();
    return;
  }

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
  // (A BME680 reading with the gas heater takes ~200 ms.)
  if (bleClientReady() && millis() - lastNotifyMs >= 1000) {
    lastNotifyMs = millis();
    String json = buildSensorJsonForTransport(false);
    if (json.indexOf("temperature") >= 0) {
      xSemaphoreTake(bleStateMutex, portMAX_DELAY);
      latestSensorJson = json;
      if (deviceConnected && authed) notifySensorJson(dataChar, json);
      xSemaphoreGive(bleStateMutex);
    }
  }
  delay(10);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Playground Sensor Node ---");

  Wire.begin();
  if (!beginBme680()) {
    Serial.println("Error: BME680 not found!");
    while (1) delay(1000);
  }
  if (!bme.setTemperatureOversampling(BME680_OS_8X) ||
      !bme.setHumidityOversampling(BME680_OS_2X) ||
      !bme.setPressureOversampling(BME680_OS_4X) ||
      !bme.setIIRFilterSize(BME680_FILTER_SIZE_3) ||
      !bme.setGasHeater(320, 150)) {
    Serial.println("Error: BME680 configuration failed!");
    while (1) delay(1000);
  }
  Serial.printf("BME680 found at I2C address 0x%02X\n", bme680I2cAddress);

  sensorReadMutex = xSemaphoreCreateMutex();
  if (sensorReadMutex == nullptr) {
    Serial.println("Sensor mutex allocation failed; restarting");
    delay(1000);
    ESP.restart();
    return;
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

  if (bme.performReading()) {
    uint8_t gasStatus = 0;
    if (!readBme680GasStatus(gasStatus)) gasStatus = 0;
    const int iaq = airQuality.updateIfValid(
        gasStatus, static_cast<int32_t>(bme.gas_resistance), bme.humidity);
    if (shortKeys) {
      doc["temp"]  = bme.temperature;
      doc["hum"]   = bme.humidity;
      doc["press"] = bme.pressure / 100.0;
      doc["iaq"]   = iaq;
    } else {
      doc["temperature"] = bme.temperature;
      doc["humidity"]    = bme.humidity;
      doc["pressure"]    = bme.pressure / 100.0;
      doc["iaq"]         = iaq;
    }
  }

  String output;
  serializeJson(doc, output);
  return output;
}

// ============================================================
// Probe both BME680 I2C addresses used by common breakout boards.
// ============================================================
bool beginBme680() {
  for (size_t i = 0;
       i < sizeof(BME680_I2C_ADDRESSES) / sizeof(BME680_I2C_ADDRESSES[0]);
       i++) {
    if (bme.begin(BME680_I2C_ADDRESSES[i])) {
      bme680I2cAddress = BME680_I2C_ADDRESSES[i];
      return true;
    }
  }
  return false;
}

// ============================================================
// Read both validity flags directly. Adafruit BME680 2.0.6 exposes no status
// and accepts either flag, while Bosch requires gas-valid AND heater-stable.
// ============================================================
bool readBme680GasStatus(uint8_t& status) {
  Wire.beginTransmission(bme680I2cAddress);
  Wire.write(BME680_GAS_STATUS_REGISTER);
  if (Wire.endTransmission(false) != 0) return false;
  if (Wire.requestFrom(bme680I2cAddress, static_cast<uint8_t>(1)) != 1) {
    return false;
  }
  status = Wire.read();
  return true;
}
