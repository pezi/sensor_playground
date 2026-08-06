#pragma once

#include <Arduino.h>
#include "mbedtls/ctr_drbg.h"
#include "mbedtls/entropy.h"
#include "mbedtls/pk.h"
#include "mbedtls/ssl.h"
#include "mbedtls/x509_crt.h"

inline bool configureSensorTls(mbedtls_ssl_config* config,
                               mbedtls_x509_crt* certificate,
                               mbedtls_pk_context* privateKey,
                               mbedtls_entropy_context* entropy,
                               mbedtls_ctr_drbg_context* random,
                               const char* certificatePem,
                               const char* privateKeyPem) {
  int result = mbedtls_ctr_drbg_seed(
      random, mbedtls_entropy_func, entropy, nullptr, 0);
  if (result != 0) {
    Serial.printf("TLS random seed failed: -0x%04X\n", -result);
    return false;
  }
  result = mbedtls_x509_crt_parse(
      certificate, reinterpret_cast<const unsigned char*>(certificatePem),
      strlen(certificatePem) + 1);
  if (result != 0) {
    Serial.printf("TLS certificate parse failed: -0x%04X\n", -result);
    return false;
  }
  result = mbedtls_pk_parse_key(
      privateKey, reinterpret_cast<const unsigned char*>(privateKeyPem),
      strlen(privateKeyPem) + 1, nullptr, 0, mbedtls_ctr_drbg_random, random);
  if (result != 0) {
    Serial.printf("TLS private-key parse failed: -0x%04X\n", -result);
    return false;
  }
  result = mbedtls_ssl_config_defaults(
      config, MBEDTLS_SSL_IS_SERVER, MBEDTLS_SSL_TRANSPORT_STREAM,
      MBEDTLS_SSL_PRESET_DEFAULT);
  if (result != 0) {
    Serial.printf("TLS defaults failed: -0x%04X\n", -result);
    return false;
  }
  mbedtls_ssl_conf_rng(config, mbedtls_ctr_drbg_random, random);
  result = mbedtls_ssl_conf_own_cert(config, certificate, privateKey);
  if (result != 0) {
    Serial.printf("TLS certificate setup failed: -0x%04X\n", -result);
    return false;
  }
  return true;
}

/// Read through the end of the HTTP headers, accumulating fragmented TLS
/// records. Returns the byte count, or a negative value on close/error/timeout.
inline int readSensorHttpRequest(mbedtls_ssl_context* ssl, char* buffer,
                                 size_t capacity,
                                 unsigned long deadlineStart,
                                 unsigned long timeoutMs) {
  size_t used = 0;
  while (used + 1 < capacity && millis() - deadlineStart < timeoutMs) {
    const int result = mbedtls_ssl_read(
        ssl, reinterpret_cast<unsigned char*>(buffer) + used,
        capacity - used - 1);
    if (result > 0) {
      used += result;
      buffer[used] = '\0';
      if (strstr(buffer, "\r\n\r\n") != nullptr) {
        return static_cast<int>(used);
      }
      continue;
    }
    if (result == MBEDTLS_ERR_SSL_WANT_READ ||
        result == MBEDTLS_ERR_SSL_WANT_WRITE) {
      delay(1);
      continue;
    }
    return result == 0 ? -1 : result;
  }
  return -1;
}

inline bool writeSensorTlsResponse(mbedtls_ssl_context* ssl,
                                   const String& response,
                                   unsigned long deadlineStart,
                                   unsigned long timeoutMs) {
  size_t written = 0;
  while (written < response.length() &&
         millis() - deadlineStart < timeoutMs) {
    const int result = mbedtls_ssl_write(
        ssl,
        reinterpret_cast<const unsigned char*>(response.c_str()) + written,
        response.length() - written);
    if (result > 0) {
      written += result;
      continue;
    }
    if (result == MBEDTLS_ERR_SSL_WANT_READ ||
        result == MBEDTLS_ERR_SSL_WANT_WRITE) {
      delay(1);
      continue;
    }
    return false;
  }
  return written == response.length();
}
