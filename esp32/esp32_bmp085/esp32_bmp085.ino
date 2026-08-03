/*
 * Sensor Tester Sensor Node — ESP32 + BMP085 barometer
 *
 * Implements the Sensor Tester Sensor Interface (see docs/sensor.md).
 * Reads a BMP085 (temperature + barometric pressure) via I2C — the sensor
 * behind the Grove Barometer Sensor, and the ancestor of the BME280 (no
 * humidity, no gas sensor). Altitude is derived from the pressure.
 * https://wiki.seeedstudio.com/Grove-Barometer_Sensor/
 *
 * The driver is written out below rather than pulled from a library: the
 * BMP085 returns *uncompensated* readings that only become values after
 * Bosch's fixed-point compensation, and the Python node has to run the
 * identical arithmetic anyway. Keeping both copies next to each other — and
 * checkable against the worked example the datasheet publishes — is worth
 * more than a dependency. See ../../python/bmp085/sensor_node.py.
 *
 * The pin-compatible BMP180 uses the same registers and works unchanged.
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
 *   ArduinoJson, Wire, and for TRANSPORT_WIFI: WiFi, WiFiUdp.
 *   (BLE uses the ESP32 core's built-in BLE stack.
 *    No sensor library — the BMP085 is driven with raw I2C reads.)
 */

// --- Transport selection (change this line) ---
#define TRANSPORT_WIFI 0
#define TRANSPORT_BLE  1
#ifndef ACTIVE_TRANSPORT
#define ACTIVE_TRANSPORT TRANSPORT_BLE
#endif

#include <Wire.h>
#include <ArduinoJson.h>
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

// ============================================================
//  BMP085 DRIVER (raw I2C, per the Bosch datasheet)
// ============================================================
const char* SENSOR_NAME = "BMP085";

// The BMP085's I2C address is fixed — there is no address pin.
const uint8_t BMP085_ADDRESS = 0x77;

const uint8_t REG_CALIBRATION = 0xAA;  // 22 bytes: AC1..AC6, B1, B2, MB, MC, MD
const uint8_t REG_CHIP_ID     = 0xD0;  // reads 0x55 on a BMP085 (and a BMP180)
const uint8_t REG_CONTROL     = 0xF4;
const uint8_t REG_DATA        = 0xF6;

const uint8_t CMD_READ_TEMPERATURE = 0x2E;
const uint8_t CMD_READ_PRESSURE    = 0x34;
const uint8_t BMP085_CHIP_ID       = 0x55;

// Oversampling (0-3): more samples, less noise, longer conversion.
const uint8_t OVERSAMPLING = 3;

// Conversion time per oversampling setting, in ms (datasheet table 3,
// rounded up for margin).
const uint8_t CONVERSION_MS[4] = {5, 8, 14, 26};

// Standard sea-level pressure, in pascal. Altitude is relative to this, so it
// moves with the weather as much as with the height.
const float SEA_LEVEL_PA = 101325.0f;

// The eleven factory constants stored in the sensor's EEPROM.
struct Bmp085Calibration {
  int16_t  ac1, ac2, ac3;
  uint16_t ac4, ac5, ac6;
  int16_t  b1, b2, mb, mc, md;
};

Bmp085Calibration gCalibration;

// Read `count` bytes starting at `reg`. Returns false on a short read, so a
// dropped sensor surfaces as a failed reading instead of stale numbers.
bool bmp085Read(uint8_t reg, uint8_t* buffer, uint8_t count) {
  Wire.beginTransmission(BMP085_ADDRESS);
  Wire.write(reg);
  if (Wire.endTransmission() != 0) return false;
  if (Wire.requestFrom((int)BMP085_ADDRESS, (int)count) != count) return false;
  for (uint8_t i = 0; i < count; i++) buffer[i] = Wire.read();
  return true;
}

bool bmp085Write(uint8_t reg, uint8_t value) {
  Wire.beginTransmission(BMP085_ADDRESS);
  Wire.write(reg);
  Wire.write(value);
  return Wire.endTransmission() == 0;
}

// Load the calibration EEPROM. Returns false if the chip is absent or the
// values are implausible.
bool bmp085Begin() {
  uint8_t chipId = 0;
  if (!bmp085Read(REG_CHIP_ID, &chipId, 1)) {
    Serial.println("Error: no response from the BMP085 address");
    return false;
  }
  if (chipId != BMP085_CHIP_ID) {
    Serial.printf("Error: chip id is 0x%02X, expected 0x%02X\n",
                  chipId, BMP085_CHIP_ID);
    return false;
  }

  uint8_t raw[22];
  if (!bmp085Read(REG_CALIBRATION, raw, sizeof(raw))) {
    Serial.println("Error: could not read the calibration data");
    return false;
  }

  uint16_t words[11];
  for (uint8_t i = 0; i < 11; i++) {
    words[i] = ((uint16_t)raw[i * 2] << 8) | raw[i * 2 + 1];
    // The datasheet states no calibration word is ever 0x0000 or 0xFFFF,
    // which is exactly what a bus with nothing powered on it reads back.
    // Catching it here beats compensating with garbage — or dividing by zero.
    if (words[i] == 0x0000 || words[i] == 0xFFFF) {
      Serial.printf("Error: implausible calibration word %u = 0x%04X\n",
                    i, words[i]);
      return false;
    }
  }

  gCalibration.ac1 = (int16_t)words[0];
  gCalibration.ac2 = (int16_t)words[1];
  gCalibration.ac3 = (int16_t)words[2];
  gCalibration.ac4 = words[3];   // AC4, AC5 and AC6 are the unsigned ones
  gCalibration.ac5 = words[4];
  gCalibration.ac6 = words[5];
  gCalibration.b1  = (int16_t)words[6];
  gCalibration.b2  = (int16_t)words[7];
  gCalibration.mb  = (int16_t)words[8];
  gCalibration.mc  = (int16_t)words[9];
  gCalibration.md  = (int16_t)words[10];
  return true;
}

// Start a temperature conversion and read the raw 16-bit result.
bool bmp085ReadRawTemperature(int32_t* out) {
  if (!bmp085Write(REG_CONTROL, CMD_READ_TEMPERATURE)) return false;
  delay(CONVERSION_MS[0]);  // temperature ignores the oversampling setting
  uint8_t data[2];
  if (!bmp085Read(REG_DATA, data, 2)) return false;
  *out = ((int32_t)data[0] << 8) | data[1];
  return true;
}

// Start a pressure conversion and read the raw oversampled result.
bool bmp085ReadRawPressure(int32_t* out) {
  if (!bmp085Write(REG_CONTROL, CMD_READ_PRESSURE + (OVERSAMPLING << 6))) {
    return false;
  }
  delay(CONVERSION_MS[OVERSAMPLING]);
  uint8_t data[3];
  if (!bmp085Read(REG_DATA, data, 3)) return false;
  int32_t raw = ((int32_t)data[0] << 16) | ((int32_t)data[1] << 8) | data[2];
  *out = raw >> (8 - OVERSAMPLING);
  return true;
}

// Turn raw readings into a temperature (°C) and a pressure (Pa).
//
// A direct transcription of the integer algorithm in the BMP085 datasheet.
// It stays in fixed point on purpose: floating point would be easier to read
// and would drift from the reference values the datasheet publishes. The
// Python node runs the same arithmetic — keep the two in step.
//
// Valid over the sensor's whole specified range (-40..+85 °C, 300..1100 hPa)
// and well beyond it; the 32-bit intermediates below only overflow past
// roughly -210 °C / +260 °C, which no reading that reaches here can produce.
void bmp085Compensate(int32_t rawTemperature, int32_t rawPressure,
                      float* temperature, int32_t* pressure) {
  const Bmp085Calibration& c = gCalibration;
  int32_t x1, x2, x3, b3, b5, b6, p;
  uint32_t b4, b7;

  // Temperature.
  x1 = (((rawTemperature - (int32_t)c.ac6) * (int32_t)c.ac5) >> 15);
  // MC is negative and shifting a negative value left is undefined behaviour
  // before C++20; the multiply below is the same arithmetic without it.
  x2 = ((int32_t)c.mc * 2048) / (x1 + (int32_t)c.md);
  b5 = x1 + x2;
  *temperature = ((b5 + 8) >> 4) / 10.0f;  // datasheet yields 0.1 °C steps

  // Pressure.
  b6 = b5 - 4000;
  x1 = ((int32_t)c.b2 * ((b6 * b6) >> 12)) >> 11;
  x2 = ((int32_t)c.ac2 * b6) >> 11;
  x3 = x1 + x2;
  b3 = ((((int32_t)c.ac1 * 4 + x3) << OVERSAMPLING) + 2) >> 2;
  x1 = ((int32_t)c.ac3 * b6) >> 13;
  x2 = ((int32_t)c.b1 * ((b6 * b6) >> 12)) >> 16;
  x3 = ((x1 + x2) + 2) >> 2;
  b4 = ((uint32_t)c.ac4 * (uint32_t)(x3 + 32768)) >> 15;
  b7 = ((uint32_t)rawPressure - b3) * (50000 >> OVERSAMPLING);
  // B7 is unsigned and can exceed 2^31, which is why it is scaled before
  // rather than after the division in that case.
  p = (b7 < 0x80000000) ? (int32_t)((b7 * 2) / b4) : (int32_t)((b7 / b4) * 2);

  // Final correction. The datasheet writes these in 32-bit ints, where the
  // squaring overflows above roughly 2150 hPa — far outside the sensor's
  // range, but signed overflow is undefined behaviour rather than merely a
  // wrong number, and it is a cliff the Python node (arbitrary-precision)
  // does not have. Widening to 64 bits costs one multiply per reading and
  // makes the two implementations agree on every input, not just plausible
  // ones.
  int64_t correction1 = (int64_t)(p >> 8) * (p >> 8);
  correction1 = (correction1 * 3038) >> 16;
  int64_t correction2 = ((int64_t)-7357 * p) >> 16;
  *pressure = (int32_t)(p + ((correction1 + correction2 + 3791) >> 4));
}

// Altitude in metres from pressure, per the international barometric formula
// the BMP085 datasheet quotes.
float bmp085Altitude(int32_t pressurePa) {
  return 44330.0f * (1.0f - pow(pressurePa / SEA_LEVEL_PA, 1.0f / 5.255f));
}

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
  if (deviceConnected && authed && millis() - lastNotifyMs >= 1000) {
    lastNotifyMs = millis();
    dataChar->setValue(buildSensorJson(false).c_str());
    dataChar->notify();
  }
  delay(10);
}
#endif

// ============================================================
void setup() {
  Serial.begin(115200);
  Serial.println("\n--- Sensor Tester Sensor Node ---");

  Wire.begin();
  if (!bmp085Begin()) {
    Serial.println("Error: BMP085 not found! (fixed I2C address 0x77)");
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

  // Temperature first, and every time: its B5 term feeds the pressure
  // compensation, so a stale one skews the pressure as the chip warms.
  int32_t rawTemperature, rawPressure;
  if (bmp085ReadRawTemperature(&rawTemperature) &&
      bmp085ReadRawPressure(&rawPressure)) {
    float temperature;
    int32_t pressurePa;
    bmp085Compensate(rawTemperature, rawPressure, &temperature, &pressurePa);

    float pressure = pressurePa / 100.0f;             // Pa -> hPa
    float altitude = bmp085Altitude(pressurePa);

    if (shortKeys) {
      doc["temp"]  = temperature;
      doc["press"] = pressure;
      doc["alt"]   = altitude;
    } else {
      doc["temperature"] = temperature;
      doc["pressure"]    = pressure;
      doc["altitude"]    = altitude;
    }
  }

  String output;
  serializeJson(doc, output);
  return output;
}
