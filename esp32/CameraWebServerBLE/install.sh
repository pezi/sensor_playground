#!/usr/bin/env bash
# Compile and upload the CameraWebServerBLE sketch to an ESP32-CAM board.
#
# Usage: ./install.sh <serial-port>
#   serial-port  device path of the connected board,
#                e.g. /dev/cu.usbserial-0001
#
# The camera node is BLE only, so unlike the sensor sketches there is no
# transport argument.

set -euo pipefail

SKETCH_DIR="$(cd "$(dirname "$0")" && pwd)"
# AI-Thinker ESP32-CAM board definition: PSRAM enabled and a 3 MB app
# partition, both of which the camera + BLE build needs. For other camera
# boards use e.g. esp32:esp32:esp32:PartitionScheme=huge_app.
FQBN="esp32:esp32:esp32cam"

if [ $# -ne 1 ]; then
  echo "Usage: $0 <serial-port>" >&2
  exit 1
fi

PORT="$1"

if [ ! -e "$PORT" ]; then
  echo "Serial port '$PORT' not found." >&2
  exit 1
fi

if [ ! -f "$SKETCH_DIR/secrets.h" ]; then
  cp "$SKETCH_DIR/secrets.h.example" "$SKETCH_DIR/secrets.h"
  echo "Created secrets.h from secrets.h.example." >&2
  echo "Edit $SKETCH_DIR/secrets.h (API key) and re-run." >&2
  exit 1
fi

arduino-cli core update-index
arduino-cli core install esp32:esp32
arduino-cli lib install "ArduinoJson"

arduino-cli compile -b "$FQBN" "$SKETCH_DIR"

arduino-cli upload -p "$PORT" --fqbn "$FQBN" "$SKETCH_DIR"

echo "Upload finished (CameraWebServerBLE)."
echo "Monitor with: arduino-cli monitor -p $PORT --config baudrate=115200"
