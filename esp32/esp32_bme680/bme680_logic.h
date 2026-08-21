#pragma once

#include <math.h>
#include <stddef.h>
#include <stdint.h>

static const uint8_t BME680_GAS_VALID_MASK = 0x20;
static const uint8_t BME680_HEAT_STABLE_MASK = 0x10;
static const uint8_t BME680_REQUIRED_GAS_STATUS =
    BME680_GAS_VALID_MASK | BME680_HEAT_STABLE_MASK;

inline bool bme680GasReadingIsValid(uint8_t status) {
  return (status & BME680_REQUIRED_GAS_STATUS) ==
         BME680_REQUIRED_GAS_STATUS;
}

// Produces the project's existing 0..100 air-quality score (higher is better).
// Only valid, heater-stable gas samples are allowed to change its baseline.
class Bme680Iaq {
 public:
  static const size_t kWindowSize = 50;

  int updateIfValid(uint8_t gasStatus, int32_t gasResistance,
                    float humidity) {
    if (!bme680GasReadingIsValid(gasStatus) || gasResistance <= 0 ||
        !isfinite(humidity)) {
      return lastIaq_;
    }

    gasData_[gasDataIndex_] = gasResistance;
    gasDataIndex_ = (gasDataIndex_ + 1) % kWindowSize;
    if (validSamples_ < kWindowSize) validSamples_++;

    int64_t sum = 0;
    for (size_t i = 0; i < kWindowSize; i++) sum += gasData_[i];
    const int32_t gasBaseline =
        static_cast<int32_t>(lround(static_cast<double>(sum) / kWindowSize));
    if (gasBaseline <= 0) return lastIaq_;

    const float boundedHumidity = clamp(humidity, 0.0f, 100.0f);
    const int32_t gasOffset = gasBaseline - gasResistance;
    const float humOffset = boundedHumidity - kHumidityBaseline;

    float humScore;
    if (humOffset > 0) {
      humScore = (100.0f - kHumidityBaseline - humOffset) /
                 (100.0f - kHumidityBaseline) *
                 (kHumidityWeight * 100.0f);
    } else {
      humScore = (kHumidityBaseline + humOffset) / kHumidityBaseline *
                 (kHumidityWeight * 100.0f);
    }

    const float gasWeight = 100.0f - (kHumidityWeight * 100.0f);
    const float gasScore = gasOffset > 0
                               ? (static_cast<float>(gasResistance) /
                                  gasBaseline) *
                                     gasWeight
                               : gasWeight;

    lastIaq_ = static_cast<int>(lround(clamp(humScore + gasScore,
                                             0.0f, 100.0f)));
    return lastIaq_;
  }

  int last() const { return lastIaq_; }
  size_t validSamples() const { return validSamples_; }
  bool baselineReady() const { return validSamples_ == kWindowSize; }

 private:
  static constexpr float kHumidityBaseline = 40.0f;
  static constexpr float kHumidityWeight = 0.25f;

  static float clamp(float value, float minimum, float maximum) {
    if (value < minimum) return minimum;
    if (value > maximum) return maximum;
    return value;
  }

  int32_t gasData_[kWindowSize] = {};
  size_t gasDataIndex_ = 0;
  size_t validSamples_ = 0;
  int lastIaq_ = 0;
};
