/*
 * Sensor Tester Sensor Node — ESP32 + Grove Dust Sensor (Shinyei PPD42NS)
 *
 * Implements the Sensor Tester Sensor Interface (see docs/sensor.md).
 * The PPD42NS pulls its output pin LOW while particles scatter light inside
 * its chamber (pulses of roughly 10-90 ms). The sketch accumulates that
 * low-pulse occupancy (LPO) over 30-second windows and converts the ratio
 * into a particle concentration in pcs/0.01cf using the Nafis curve,
 * reported under the JSON key `dust`:
 *
 *   ratio         = low_time / window_time * 100          (percent)
 *   concentration = 1.1*r^3 - 3.8*r^2 + 520*r + 0.62      (pcs/0.01cf)
 *
 * https://wiki.seeedstudio.com/Grove-Dust_Sensor/
 * https://www.howmuchsnow.com/arduino/airquality/grovedust/
 *
 * Unlike the digital contact sensors this is a *pollable* node (a continuous
 * value), so it uses the same HTTPS REST + UDP discovery transport as the
 * environment sensors rather than the WebSocket push path. The first reading
 * appears after the first full 30-second window; until then the payloads
 * carry no `dust` key.
 *
 * Transport is chosen at compile time via ACTIVE_TRANSPORT:
 *   TRANSPORT_WIFI — UDP discovery (9133) + HTTPS REST (9132), self-signed cert
 *   TRANSPORT_BLE  — BLE GATT service; the app scans for SERVICE_UUID, writes
 *                    the API key to AUTH_CHAR_UUID, then reads/subscribes
 *                    DATA_CHAR_UUID for the same JSON payload.
 *
 * Required Libraries:
 *   ArduinoJson, and for TRANSPORT_WIFI: WiFi, WiFiUdp.
 *   (No sensor library — the pulses are timed with a pin-change interrupt.)
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
const char* SENSOR_NAME = "PPD42NS";

// Pin the PPD42NS P1 output (yellow) reaches through a voltage divider — the
// sensor runs on 5 V and its output swings up to ~4.5 V, which is NOT
// 3.3 V-safe (see README). GPIO 34 is input-only; the divider drives it, so
// no internal pull is needed.
const int DUST_PIN = 34;

const unsigned long DUST_WINDOW_MS = 30000;

// Written by the ISR, read under noInterrupts() in updateDustWindow().
volatile unsigned long dustLpoUs = 0;       // low time accumulated this window
volatile unsigned long dustLowStartUs = 0;  // micros() of the falling edge
volatile bool dustPinLow = false;           // inside a low pulse right now?

unsigned long dustWindowStartMs = 0;        // loop()-only, no ISR access
float dustConcentration = NAN;              // NAN until the first window closes

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

// Pulses are timed with an edge interrupt instead of pulseIn(): pulseIn()
// blocks for the length of a pulse and sees nothing in between, and the TLS
// handshake can hold loop() for several seconds — the ISR keeps accumulating
// low time regardless of what loop() is doing.
void IRAM_ATTR dustIsr() {
  unsigned long now = micros();
  if (digitalRead(DUST_PIN) == LOW) {       // falling edge: pulse starts
    dustLowStartUs = now;
    dustPinLow = true;
  } else if (dustPinLow) {                  // rising edge: pulse ends
    dustLpoUs += now - dustLowStartUs;      // unsigned math survives rollover
    dustPinLow = false;
  }
}

// Called every loop() pass; closes the LPO window once it is due and caches
// the concentration. A rollover delayed by a long TLS handshake stays correct
// because the ratio divides by the *actual* elapsed time, not the nominal
// window length.
void updateDustWindow() {
  unsigned long nowMs = millis();
  unsigned long elapsedMs = nowMs - dustWindowStartMs;
  if (elapsedMs < DUST_WINDOW_MS) return;

  noInterrupts();                           // consistent snapshot + reset
  unsigned long lpoUs = dustLpoUs;
  dustLpoUs = 0;
  if (dustPinLow) {
    // A pulse spans the boundary: credit the elapsed part to the closing
    // window and restart the low-timer for the new one.
    unsigned long nowUs = micros();
    lpoUs += nowUs - dustLowStartUs;
    dustLowStartUs = nowUs;
  }
  interrupts();
  dustWindowStartMs = nowMs;

  float ratio = (float)lpoUs / (elapsedMs * 1000.0f) * 100.0f;
  dustConcentration =
      1.1f * ratio * ratio * ratio - 3.8f * ratio * ratio + 520.0f * ratio + 0.62f;

  Serial.printf("LPO %.2f %% -> %.1f pcs/0.01cf\n", ratio, dustConcentration);
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
  // Push a fresh reading once per second to subscribed, authorized clients —
  // but only once the first LPO window has produced a value (the payload
  // carries no "dust" key during warm-up).
  if (deviceConnected && authed && millis() - lastNotifyMs >= 1000) {
    lastNotifyMs = millis();
    String json = buildSensorJsonForTransport(false);
    if (json.indexOf("dust") >= 0) {
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

  pinMode(DUST_PIN, INPUT);
  attachInterrupt(digitalPinToInterrupt(DUST_PIN), dustIsr, CHANGE);
  // The ISR only sees edges: a pin that is already low (a stuck or
  // misbehaving line) would read as 0 % occupancy — "perfectly clean air" —
  // instead of saturation. Treat an initial low as a pulse in progress so a
  // stuck-low line reports a huge value, not a clean one.
  if (digitalRead(DUST_PIN) == LOW) {
    dustLowStartUs = micros();
    dustPinLow = true;
  }
  dustWindowStartMs = millis();
  Serial.println("First reading after the first full 30-second LPO window.");

  transportSetup();

  Serial.print("Sensor: ");
  Serial.println(SENSOR_NAME);
}

// ============================================================
void loop() {
  lockSensorStateForTransport();
  updateDustWindow();
  unlockSensorStateForTransport();
  transportLoop();
}

// ============================================================
// Build sensor JSON (identical payload on every transport).
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
  } else {
    doc["sensor"] = SENSOR_NAME;
    doc["host"]   = HOSTNAME;
  }

  // Same short/long key ("dust") on both the discovery and REST payloads.
  // Omitted until the first 30-second LPO window has completed.
  if (!isnan(dustConcentration)) doc["dust"] = dustConcentration;

  String output;
  serializeJson(doc, output);
  return output;
}
