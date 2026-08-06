/*
 * Sensor Tester Sensor Node — ESP32 + Air530 GPS
 *
 * Implements the Sensor Tester Sensor Interface (see docs/sensor.md).
 * Reads a Grove GPS (Air530) module (latitude, longitude, altitude,
 * satellites in use). Like the CozIR this is a serial device, not I2C:
 * it continuously streams NMEA-0183 sentences over a 9600-baud UART.
 *
 * The sketch parses each complete GGA sentence in loop() and caches the
 * latest valid fix (the Air530 emits GNGGA once per second, so the GLL
 * fallback of the dart_periphery NmeaParser is not needed here — see
 * serial_air530.dart). Readings older than 5 s are not served, e.g.
 * while the module is still searching for a fix.
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
 *   ArduinoJson,
 *   and for TRANSPORT_WIFI: WiFi, WiFiUdp.
 *   (BLE uses the ESP32 core's built-in BLE stack; the Air530 needs no
 *   sensor library — it is read over Serial2.)
 */

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

  // Shared Sensor Tester GATT contract (must match the app's BleUuids).
  #define SERVICE_UUID   "d1a51b00-0001-4a7e-9b3c-0a1b2c3d4e5f"
  #define DATA_CHAR_UUID "d1a51b00-0002-4a7e-9b3c-0a1b2c3d4e5f"
  #define AUTH_CHAR_UUID "d1a51b00-0003-4a7e-9b3c-0a1b2c3d4e5f"
#endif

// --- Sensor ---
const char* SENSOR_NAME = "AIR530";

// Air530 on UART2.
const int GPS_RX_PIN = 16;  // ESP32 RX2 <- module Tx
const int GPS_TX_PIN = 17;  // ESP32 TX2 -> module Rx

// Latest valid fix, cached by pollGps(); readings older than this are
// not served (e.g. the module lost its fix).
const unsigned long FIX_MAX_AGE_MS = 5000;

double gLatitude = 0;
double gLongitude = 0;
double gAltitude = 0;
int gSatellites = 0;
bool gHasAltitude = false;
bool gHasSatellites = false;
unsigned long gFixMs = 0;

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

bool hasRecentFix() {
  return gFixMs != 0 && millis() - gFixMs < FIX_MAX_AGE_MS;
}

// ============================================================
//  NMEA parsing (per the dart_periphery NmeaParser)
// ============================================================

// Verify the XOR checksum between '$' and '*'.
bool nmeaChecksumOk(const char* line) {
  const char* star = strrchr(line, '*');
  if (star == NULL || star == line || strlen(star) < 3) return false;
  uint8_t calc = 0;
  for (const char* p = line + 1; p < star; ++p) calc ^= (uint8_t)*p;
  return strtol(star + 1, NULL, 16) == calc;
}

// Convert NMEA ddmm.mmmm (lat) / dddmm.mmmm (lon) + hemisphere to
// decimal degrees.
bool nmeaCoordToDecimal(const char* coord, const char* hemi, double* out) {
  if (coord[0] == '\0' || hemi[0] == '\0') return false;
  double value = atof(coord);
  int degrees = (int)(value / 100.0);
  double decimal = degrees + (value - degrees * 100.0) / 60.0;
  if (hemi[0] == 'S' || hemi[0] == 'W') decimal = -decimal;
  *out = decimal;
  return true;
}

// Split a sentence body in place at each ',' (NMEA fields may be empty).
int splitNmeaFields(char* body, const char* fields[], int maxFields) {
  int count = 0;
  fields[count++] = body;
  for (char* p = body; *p != '\0' && count < maxFields; ++p) {
    if (*p == ',') {
      *p = '\0';
      fields[count++] = p + 1;
    }
  }
  return count;
}

// Parse one complete NMEA line; cache the fix if it is a valid GGA.
// GGA indices: 0 GxGGA, 1 time, 2 lat, 3 N/S, 4 lon, 5 E/W,
// 6 fix quality (0 = no fix), 7 sats used, 8 HDOP, 9 alt MSL, ...
void handleNmeaLine(char* line) {
  if (line[0] != '$' || !nmeaChecksumOk(line)) return;
  *strrchr(line, '*') = '\0';  // drop the checksum

  const char* fields[24];
  int count = splitNmeaFields(line + 1, fields, 24);

  size_t talkerLen = strlen(fields[0]);
  if (talkerLen < 5 || strcmp(fields[0] + talkerLen - 3, "GGA") != 0) return;
  if (count < 10) return;
  if (atoi(fields[6]) == 0) return;  // no valid fix

  double latitude, longitude;
  if (!nmeaCoordToDecimal(fields[2], fields[3], &latitude)) return;
  if (!nmeaCoordToDecimal(fields[4], fields[5], &longitude)) return;

  gLatitude = latitude;
  gLongitude = longitude;
  gHasAltitude = fields[9][0] != '\0';
  if (gHasAltitude) gAltitude = atof(fields[9]);
  gHasSatellites = fields[7][0] != '\0';
  if (gHasSatellites) gSatellites = atoi(fields[7]);
  gFixMs = millis();
}

// Assemble complete lines from the NMEA stream and parse each one.
void pollGps() {
  static char line[128];
  static size_t len = 0;

  while (Serial2.available()) {
    char c = Serial2.read();
    if (c == '\n') {
      line[len] = '\0';
      handleNmeaLine(line);
      len = 0;
    } else if (c != '\r') {
      if (len < sizeof(line) - 1) {
        line[len++] = c;
      } else {
        len = 0;  // overlong garbage — drop the line
      }
    }
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
  // Readings without a recent fix are skipped (buildSensorJson omits the
  // value fields while the module is still searching).
  if (deviceConnected && authed && millis() - lastNotifyMs >= 1000) {
    lastNotifyMs = millis();
    String json = buildSensorJsonForTransport(false);
    if (json.indexOf("latitude") >= 0) {
      notifySensorJson(dataChar, json);
    }
  }
  delay(10);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  Serial2.begin(9600, SERIAL_8N1, GPS_RX_PIN, GPS_TX_PIN);
  // The module streams NMEA continuously — seeing a '$' within a few
  // seconds confirms it is wired up (a position fix may take longer).
  bool found = false;
  unsigned long start = millis();
  while (millis() - start < 3000) {
    if (Serial2.available() && Serial2.read() == '$') {
      found = true;
      break;
    }
  }
  if (!found) {
    Serial.println("Error: Air530 not found!");
    while (1) delay(1000);
  }

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  lockSensorStateForTransport();
  pollGps();
  unlockSensorStateForTransport();
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

  if (hasRecentFix()) {
    if (shortKeys) {
      doc["lat"] = gLatitude;
      doc["lon"] = gLongitude;
      if (gHasAltitude) doc["alt"] = gAltitude;
      if (gHasSatellites) doc["sats"] = gSatellites;
    } else {
      doc["latitude"] = gLatitude;
      doc["longitude"] = gLongitude;
      if (gHasAltitude) doc["altitude"] = gAltitude;
      if (gHasSatellites) doc["satellites"] = gSatellites;
    }
  }

  String output;
  serializeJson(doc, output);
  return output;
}
