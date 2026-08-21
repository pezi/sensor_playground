#!/usr/bin/env bash
# Compile and upload the esp32_bme680 sketch to an ESP32 dev board.
#
# Usage: ./install.sh <serial-port> [WIFI|BLE]
#   serial-port  device path of the connected ESP32 board,
#                e.g. /dev/cu.usbserial-0001
#   transport    optional: WIFI or BLE (default: BLE)

set -euo pipefail

SKETCH_DIR="$(cd "$(dirname "$0")" && pwd)"
FQBN="esp32:esp32:esp32"
ESP32_CORE_VERSION="3.3.11"
ARDUINO_JSON_VERSION="7.4.3"
ADAFRUIT_BME680_VERSION="2.0.6"
ADAFRUIT_BUSIO_VERSION="1.17.4"
ADAFRUIT_UNIFIED_SENSOR_VERSION="1.1.15"

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
  echo "Usage: $0 <serial-port> [WIFI|BLE]" >&2
  exit 1
fi

PORT="$1"
TRANSPORT="$(printf '%s' "${2:-BLE}" | tr '[:lower:]' '[:upper:]')"

case "$TRANSPORT" in
  WIFI|BLE) ;;
  *)
    echo "Invalid transport '$2' — use WIFI or BLE." >&2
    exit 1
    ;;
esac

if [ ! -e "$PORT" ]; then
  echo "Serial port '$PORT' not found." >&2
  exit 1
fi

if [ ! -f "$SKETCH_DIR/secrets.h" ]; then
  cp "$SKETCH_DIR/secrets.h.example" "$SKETCH_DIR/secrets.h"
  echo "Created secrets.h from secrets.h.example." >&2
  echo "Edit $SKETCH_DIR/secrets.h (Wi-Fi credentials, API key) and re-run." >&2
  exit 1
fi

arduino-cli core update-index
arduino-cli core install "esp32:esp32@$ESP32_CORE_VERSION"
arduino-cli lib install \
  "ArduinoJson@$ARDUINO_JSON_VERSION" \
  "Adafruit BusIO@$ADAFRUIT_BUSIO_VERSION" \
  "Adafruit Unified Sensor@$ADAFRUIT_UNIFIED_SENSOR_VERSION" \
  "Adafruit BME680 Library@$ADAFRUIT_BME680_VERSION"

arduino-cli compile -b "$FQBN" \
  --build-property "compiler.cpp.extra_flags=-DACTIVE_TRANSPORT=TRANSPORT_$TRANSPORT" \
  "$SKETCH_DIR"

arduino-cli upload -p "$PORT" --fqbn "$FQBN" "$SKETCH_DIR"

echo "Upload finished (esp32_bme680, $TRANSPORT)."
echo "Monitor with: arduino-cli monitor -p $PORT --config baudrate=115200"
