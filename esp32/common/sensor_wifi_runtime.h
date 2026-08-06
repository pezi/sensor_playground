#pragma once

#include <Arduino.h>
#include <WiFi.h>

static const unsigned long SENSOR_WIFI_CONNECT_TIMEOUT_MS = 30000;
static const unsigned long SENSOR_WIFI_RETRY_INTERVAL_MS = 5000;

/// Connect during setup with a finite deadline. The caller decides whether to
/// restart or enter a provisioning/error state when this returns false.
inline bool connectSensorWifi(const char* ssid, const char* password) {
  WiFi.mode(WIFI_STA);
  WiFi.begin(ssid, password);
  Serial.print("Connecting to WiFi");
  const unsigned long started = millis();
  while (WiFi.status() != WL_CONNECTED &&
         millis() - started < SENSOR_WIFI_CONNECT_TIMEOUT_MS) {
    delay(500);
    Serial.print(".");
  }
  if (WiFi.status() != WL_CONNECTED) {
    Serial.println("\nWiFi connection timed out");
    return false;
  }
  Serial.println("\nWiFi connected.");
  Serial.print("IP: ");
  Serial.println(WiFi.localIP());
  return true;
}

/// Non-blocking runtime reconnect with backoff. Transport loops return early
/// while the link is down rather than operating dead sockets indefinitely.
inline bool sensorWifiReady() {
  if (WiFi.status() == WL_CONNECTED) return true;
  static unsigned long lastAttempt = 0;
  const unsigned long now = millis();
  if (lastAttempt == 0 || now - lastAttempt >= SENSOR_WIFI_RETRY_INTERVAL_MS) {
    lastAttempt = now;
    Serial.println("WiFi disconnected; reconnecting");
    WiFi.reconnect();
  }
  return false;
}
