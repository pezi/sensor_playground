"""Reliability fixes around the pinned Pimoroni ``bme680`` driver.

The upstream driver starts a forced conversion but only polls for 100 ms.  The
profile used by this node needs about 183 ms, including its 150 ms gas-heater
phase.  It also decodes Bosch's signed four-bit range-switch error as unsigned.

This adapter deliberately uses the private register/compensation helpers from
Pimoroni bme680 2.0.0.  requirements.txt pins that version so these internals
cannot change underneath us.
"""

import time


_FIELD0_ADDR = 0x1D
_MEAS_INDEX_ADDR = 0x1E
_FIELD_LENGTH = 17
_RANGE_SW_ERR_ADDR = 0x04

_NEW_DATA_MASK = 0x80
_GAS_INDEX_MASK = 0x0F
_GAS_RANGE_MASK = 0x0F
_GAS_VALID_MASK = 0x20
_HEAT_STABLE_MASK = 0x10

_FORCED_MODE = 0x01
_VARIANT_HIGH = 0x01
_POLL_INTERVAL_SECONDS = 0.005
_TIMING_MARGIN_SECONDS = 0.050
_OVERSAMPLING_CYCLES = (0, 1, 2, 4, 8, 16)


def decode_range_switch_error(raw_register):
    """Decode register 0x04 bits 7:4 as Bosch's signed four-bit value."""
    value = (raw_register >> 4) & 0x0F
    return value - 16 if value & 0x08 else value


def measurement_timeout_seconds(
    temperature_oversampling,
    pressure_oversampling,
    humidity_oversampling,
    heater_duration_ms,
):
    """Return a forced-mode deadline using Bosch's measurement-duration formula."""
    oversampling = (
        temperature_oversampling,
        pressure_oversampling,
        humidity_oversampling,
    )
    if any(
        not isinstance(value, int)
        or not 0 <= value < len(_OVERSAMPLING_CYCLES)
        for value in oversampling
    ):
        raise ValueError("invalid BME680 oversampling setting")
    if heater_duration_ms < 0:
        raise ValueError("invalid BME680 heater duration")

    cycles = sum(_OVERSAMPLING_CYCLES[value] for value in oversampling)

    tph_microseconds = cycles * 1963 + 477 * 4 + 477 * 5 + 1000
    return (
        tph_microseconds / 1_000_000.0
        + heater_duration_ms / 1000.0
        + _TIMING_MARGIN_SECONDS
    )


class PimoroniBME680Adapter:
    """Read fresh forced-mode samples and correct affected calibration data."""

    def __init__(
        self,
        sensor,
        *,
        temperature_oversampling,
        pressure_oversampling,
        humidity_oversampling,
        heater_duration_ms,
        monotonic=time.monotonic,
        sleep=time.sleep,
    ):
        self._sensor = sensor
        self._monotonic = monotonic
        self._sleep = sleep
        self._timeout = measurement_timeout_seconds(
            temperature_oversampling,
            pressure_oversampling,
            humidity_oversampling,
            heater_duration_ms,
        )

        raw_sw_error = sensor._get_regs(_RANGE_SW_ERR_ADDR, 1)
        sensor.calibration_data.range_sw_err = decode_range_switch_error(
            raw_sw_error
        )

    @property
    def data(self):
        return self._sensor.data

    def get_sensor_data(self):
        """Populate ``data`` only after the measurement index advances."""
        previous_index = self._sensor._get_regs(_MEAS_INDEX_ADDR, 1)
        self._sensor.set_power_mode(_FORCED_MODE)
        deadline = self._monotonic() + self._timeout

        while self._monotonic() <= deadline:
            status = self._sensor._get_regs(_FIELD0_ADDR, 1)
            if status & _NEW_DATA_MASK:
                registers = self._sensor._get_regs(_FIELD0_ADDR, _FIELD_LENGTH)
                if registers[1] != previous_index:
                    self._decode(registers)
                    return True
            self._sleep(_POLL_INTERVAL_SECONDS)

        return False

    def _decode(self, registers):
        data = self._sensor.data
        data.status = registers[0] & _NEW_DATA_MASK
        data.gas_index = registers[0] & _GAS_INDEX_MASK
        data.meas_index = registers[1]

        adc_pressure = (
            (registers[2] << 12) | (registers[3] << 4) | (registers[4] >> 4)
        )
        adc_temperature = (
            (registers[5] << 12) | (registers[6] << 4) | (registers[7] >> 4)
        )
        adc_humidity = (registers[8] << 8) | registers[9]

        if self._sensor._variant == _VARIANT_HIGH:
            adc_gas = (registers[15] << 2) | (registers[16] >> 6)
            gas_range = registers[16] & _GAS_RANGE_MASK
            gas_status = registers[16]
            gas_resistance = self._sensor._calc_gas_resistance_high(
                adc_gas, gas_range
            )
        else:
            adc_gas = (registers[13] << 2) | (registers[14] >> 6)
            gas_range = registers[14] & _GAS_RANGE_MASK
            gas_status = registers[14]
            gas_resistance = self._sensor._calc_gas_resistance_low(
                adc_gas, gas_range
            )

        data.status |= gas_status & (_GAS_VALID_MASK | _HEAT_STABLE_MASK)
        data.gas_valid = bool(data.status & _GAS_VALID_MASK)
        data.heat_stable = bool(data.status & _HEAT_STABLE_MASK)

        temperature = self._sensor._calc_temperature(adc_temperature)
        data.temperature = temperature / 100.0
        self._sensor.ambient_temperature = temperature
        data.pressure = self._sensor._calc_pressure(adc_pressure) / 100.0
        data.humidity = self._sensor._calc_humidity(adc_humidity) / 1000.0
        data.gas_resistance = gas_resistance
