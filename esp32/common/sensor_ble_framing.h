#pragma once

#include <Arduino.h>
#include <BLEDevice.h>

// Notification frame: marker, message id, chunk index, flags, content. Keeping
// every packet at 20 bytes makes it safe at BLE's mandatory 23-byte ATT MTU.
static const uint8_t SENSOR_BLE_FRAME_MARKER = 0x1E;
static const uint8_t SENSOR_BLE_FRAME_START = 0x01;
static const uint8_t SENSOR_BLE_FRAME_END = 0x02;
static const size_t SENSOR_BLE_FRAME_HEADER = 4;
static const size_t SENSOR_BLE_FRAME_CONTENT = 20 - SENSOR_BLE_FRAME_HEADER;

inline void notifySensorJson(BLECharacteristic* characteristic,
                             const String& json) {
  static uint8_t messageId = 0;
  messageId++;

  const uint8_t* content =
      reinterpret_cast<const uint8_t*>(json.c_str());
  const size_t length = json.length();
  uint8_t packet[20];
  uint8_t chunkIndex = 0;

  for (size_t offset = 0; offset < length;
       offset += SENSOR_BLE_FRAME_CONTENT, chunkIndex++) {
    const size_t count =
        min(SENSOR_BLE_FRAME_CONTENT, length - offset);
    uint8_t flags = 0;
    if (offset == 0) flags |= SENSOR_BLE_FRAME_START;
    if (offset + count == length) flags |= SENSOR_BLE_FRAME_END;

    packet[0] = SENSOR_BLE_FRAME_MARKER;
    packet[1] = messageId;
    packet[2] = chunkIndex;
    packet[3] = flags;
    memcpy(packet + SENSOR_BLE_FRAME_HEADER, content + offset, count);
    characteristic->setValue(packet, SENSOR_BLE_FRAME_HEADER + count);
    characteristic->notify();
    // Give the controller time to queue this notification before its value is
    // replaced by the next chunk.
    delay(5);
  }
}
