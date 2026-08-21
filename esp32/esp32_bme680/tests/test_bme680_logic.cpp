#include <assert.h>
#include <math.h>
#include <stdint.h>

#include "../bme680_logic.h"

int main() {
  assert(!bme680GasReadingIsValid(0));
  assert(!bme680GasReadingIsValid(BME680_GAS_VALID_MASK));
  assert(!bme680GasReadingIsValid(BME680_HEAT_STABLE_MASK));
  assert(bme680GasReadingIsValid(BME680_REQUIRED_GAS_STATUS));

  Bme680Iaq iaq;
  assert(iaq.updateIfValid(BME680_GAS_VALID_MASK, 120000, 45.0f) == 0);
  assert(iaq.updateIfValid(BME680_HEAT_STABLE_MASK, 120000, 45.0f) == 0);
  assert(iaq.updateIfValid(BME680_REQUIRED_GAS_STATUS, 0, 45.0f) == 0);
  assert(iaq.updateIfValid(BME680_REQUIRED_GAS_STATUS, 120000, NAN) == 0);
  assert(iaq.validSamples() == 0);

  const int first = iaq.updateIfValid(
      BME680_REQUIRED_GAS_STATUS, 120000, 45.0f);
  assert(first > 0 && first <= 100);
  assert(iaq.validSamples() == 1);
  assert(!iaq.baselineReady());

  const int retained = iaq.updateIfValid(0, 1, 90.0f);
  assert(retained == first);
  assert(iaq.validSamples() == 1);

  for (size_t i = 1; i < Bme680Iaq::kWindowSize; i++) {
    iaq.updateIfValid(BME680_REQUIRED_GAS_STATUS, 120000, 40.0f);
  }
  assert(iaq.baselineReady());
  assert(iaq.validSamples() == Bme680Iaq::kWindowSize);
  assert(iaq.last() == 100);

  // The sample counter stays bounded with the rolling window.
  iaq.updateIfValid(BME680_REQUIRED_GAS_STATUS, 120000, 40.0f);
  assert(iaq.validSamples() == Bme680Iaq::kWindowSize);

  // Out-of-range humidity is bounded, keeping the public score in 0..100.
  const int dry = iaq.updateIfValid(
      BME680_REQUIRED_GAS_STATUS, 120000, -20.0f);
  const int wet = iaq.updateIfValid(
      BME680_REQUIRED_GAS_STATUS, 120000, 120.0f);
  assert(dry >= 0 && dry <= 100);
  assert(wet >= 0 && wet <= 100);
  return 0;
}
