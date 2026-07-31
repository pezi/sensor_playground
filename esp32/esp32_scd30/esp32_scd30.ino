/*
 * Sensor Tester Sensor Node — ESP32 + SCD30
 *
 * Implements the Sensor Tester Sensor Interface (see docs/sensor.md).
 * Reads an SCD30 (temperature, humidity, CO2) via I2C.
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
 *   ArduinoJson, Wire, SparkFun SCD30 Arduino Library,
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
#include <SparkFun_SCD30_Arduino_Library.h>
#include "secrets.h"

#if ACTIVE_TRANSPORT == TRANSPORT_WIFI
  #include <WiFi.h>
  #include <WiFiUdp.h>
  #include "mbedtls/ssl.h"
  #include "mbedtls/pk.h"
  #include "mbedtls/x509_crt.h"
  #include "mbedtls/entropy.h"
  #include "mbedtls/ctr_drbg.h"
  #include "mbedtls/net_sockets.h"
  #include "mbedtls/error.h"
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

// --- Sensor ---
SCD30 scd30;
const char* SENSOR_NAME = "SCD30";

String buildSensorJson(bool shortKeys);

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

  mbedtls_ssl_config_init(&sslConf);
  mbedtls_x509_crt_init(&srvcert);
  mbedtls_pk_init(&pkey);
  mbedtls_entropy_init(&entropy);
  mbedtls_ctr_drbg_init(&ctr_drbg);

  mbedtls_ctr_drbg_seed(&ctr_drbg, mbedtls_entropy_func, &entropy, NULL, 0);
  int ret = mbedtls_x509_crt_parse(&srvcert, (const unsigned char*)SERVER_CERT, strlen(SERVER_CERT) + 1);
  if (ret != 0) {
    Serial.printf("Error parsing certificate: -0x%04X\n", -ret);
  }
  ret = mbedtls_pk_parse_key(&pkey, (const unsigned char*)SERVER_KEY, strlen(SERVER_KEY) + 1, NULL, 0, mbedtls_ctr_drbg_random, &ctr_drbg);
  if (ret != 0) {
    Serial.printf("Error parsing private key: -0x%04X\n", -ret);
  }

  mbedtls_ssl_config_defaults(&sslConf, MBEDTLS_SSL_IS_SERVER,
    MBEDTLS_SSL_TRANSPORT_STREAM, MBEDTLS_SSL_PRESET_DEFAULT);
  mbedtls_ssl_conf_rng(&sslConf, mbedtls_ctr_drbg_random, &ctr_drbg);
  mbedtls_ssl_conf_own_cert(&sslConf, &srvcert, &pkey);

  tcpServer.begin();
  udp.begin(UDP_PORT);
  Serial.println("HTTPS on port 9132, UDP on port 9133");
}

void transportLoop() {
  handleUdpDiscovery();
  handleTlsClient();
  delay(1);
}

void handleUdpDiscovery() {
  int packetSize = udp.parsePacket();
  if (!packetSize) return;

  char buffer[64];
  int len = udp.read(buffer, sizeof(buffer) - 1);
  buffer[len] = '\0';
  if (strstr(buffer, "SENSOR_TESTER") == NULL) return;

  String json = buildSensorJson(true);
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

void handleTlsClient() {
  WiFiClient client = tcpServer.accept();
  if (!client) return;

  mbedtls_ssl_context ssl;
  mbedtls_ssl_init(&ssl);
  mbedtls_ssl_setup(&ssl, &sslConf);
  mbedtls_ssl_set_bio(&ssl, &client, tlsSend, tlsRecv, NULL);

  // Overall deadline for the whole client interaction: tlsRecv only bounds a
  // single read, so without this a stalled client blocks loop() - and with it
  // UDP discovery - indefinitely.
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
  int reqLen = 0;
  do {
    ret = mbedtls_ssl_read(&ssl, (unsigned char*)reqBuf + reqLen, sizeof(reqBuf) - reqLen - 1);
  } while (ret == MBEDTLS_ERR_SSL_WANT_READ && millis() - tlsStart < 4000);
  if (ret > 0) reqLen = ret;
  reqBuf[reqLen] = '\0';

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
  if (apiKey != String(API_KEY)) {
    response = "HTTP/1.1 401 Unauthorized\r\nConnection: close\r\n\r\n";
  } else {
    String json = buildSensorJson(false);
    response = "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n" + json;
  }

  mbedtls_ssl_write(&ssl, (const unsigned char*)response.c_str(), response.length());
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
    characteristic->setValue(authed ? buildSensorJson(false).c_str() : "{}");
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
  // The SCD30 measures every ~2 s; readings without fresh data are skipped
  // (buildSensorJson omits the value fields when none are available).
  if (deviceConnected && authed && millis() - lastNotifyMs >= 1000) {
    lastNotifyMs = millis();
    String json = buildSensorJson(false);
    if (json.indexOf("temperature") >= 0) {
      dataChar->setValue(json.c_str());
      dataChar->notify();
    }
  }
  delay(10);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  Wire.begin();
  if (!scd30.begin()) {
    Serial.println("Error: SCD30 not found!");
    while (1) delay(1000);
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

  if (scd30.dataAvailable()) {
    if (shortKeys) {
      doc["temp"] = scd30.getTemperature();
      doc["hum"]  = scd30.getHumidity();
      doc["co2"]  = scd30.getCO2();
    } else {
      doc["temperature"] = scd30.getTemperature();
      doc["humidity"]    = scd30.getHumidity();
      doc["co2"]         = (double)scd30.getCO2();
    }
  }

  String output;
  serializeJson(doc, output);
  return output;
}
