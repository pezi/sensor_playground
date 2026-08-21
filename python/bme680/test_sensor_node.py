"""Regression tests for the Python BME680 reliability fixes."""

import threading
import time
import unittest
from concurrent.futures import ThreadPoolExecutor
from types import SimpleNamespace

from bme680_reliable import (
    PimoroniBME680Adapter,
    decode_range_switch_error,
    measurement_timeout_seconds,
)
from sensor_node import BME680Sensor


class _FakeClock:
    def __init__(self):
        self.now = 0.0

    def monotonic(self):
        return self.now

    def sleep(self, seconds):
        self.now += seconds


class _FakePimoroniSensor:
    def __init__(self, clock):
        self._clock = clock
        self._variant = 0
        self.calibration_data = SimpleNamespace(range_sw_err=12)
        self.data = SimpleNamespace()
        self.ambient_temperature = 0
        self.field_reads = 0
        self.power_modes = []

    def _get_regs(self, address, length):
        if address == 0x04:
            return 0xC3
        if address == 0x1E:
            return 7
        if address == 0x1D and length == 1:
            return 0x80 if self._clock.now >= 0.18 else 0
        if address == 0x1D and length == 17:
            self.field_reads += 1
            registers = [0] * 17
            registers[0] = 0x80
            # The first ready field is deliberately stale. Only the second
            # read advances the measurement index and may be accepted.
            registers[1] = 7 if self.field_reads == 1 else 8
            registers[14] = 0x30
            return registers
        raise AssertionError(f"unexpected register read: {address:#x}/{length}")

    def set_power_mode(self, mode):
        self.power_modes.append(mode)

    def _calc_temperature(self, _adc):
        return 2200

    def _calc_pressure(self, _adc):
        return 101325

    def _calc_humidity(self, _adc):
        return 45000

    def _calc_gas_resistance_low(self, _adc, _gas_range):
        return 120000

    def _calc_gas_resistance_high(self, _adc, _gas_range):
        return 120000


class _FakeReadingAdapter:
    def __init__(self, *, gas_valid=True, heat_stable=True, delay=0.0):
        self.data = SimpleNamespace(
            temperature=22.0,
            humidity=45.0,
            pressure=1013.25,
            gas_resistance=120000,
            gas_valid=gas_valid,
            heat_stable=heat_stable,
        )
        self._delay = delay
        self._active = 0
        self.max_active = 0
        self._state_lock = threading.Lock()

    def get_sensor_data(self):
        with self._state_lock:
            self._active += 1
            self.max_active = max(self.max_active, self._active)
        time.sleep(self._delay)
        with self._state_lock:
            self._active -= 1
        return True


def _sensor_with(adapter):
    sensor = BME680Sensor.__new__(BME680Sensor)
    sensor.sensor = adapter
    sensor._initialize_state()
    sensor.name = "BME680"
    return sensor


class BME680ReliabilityTest(unittest.TestCase):
    def test_signed_range_switch_error(self):
        self.assertEqual(decode_range_switch_error(0xC3), -4)
        self.assertEqual(decode_range_switch_error(0x73), 7)

    def test_waits_for_profile_and_rejects_stale_measurement_index(self):
        clock = _FakeClock()
        raw_sensor = _FakePimoroniSensor(clock)
        adapter = PimoroniBME680Adapter(
            raw_sensor,
            temperature_oversampling=4,
            pressure_oversampling=3,
            humidity_oversampling=2,
            heater_duration_ms=150,
            monotonic=clock.monotonic,
            sleep=clock.sleep,
        )

        self.assertGreater(
            measurement_timeout_seconds(4, 3, 2, 150), 0.18
        )
        self.assertTrue(adapter.get_sensor_data())
        self.assertGreaterEqual(clock.now, 0.18)
        self.assertEqual(raw_sensor.field_reads, 2)
        self.assertEqual(adapter.data.meas_index, 8)
        self.assertTrue(adapter.data.gas_valid)
        self.assertTrue(adapter.data.heat_stable)
        self.assertEqual(raw_sensor.calibration_data.range_sw_err, -4)

    def test_invalid_gas_does_not_advance_iaq_baseline(self):
        for gas_valid, heat_stable in ((False, True), (True, False)):
            with self.subTest(
                gas_valid=gas_valid, heat_stable=heat_stable
            ):
                adapter = _FakeReadingAdapter(
                    gas_valid=gas_valid, heat_stable=heat_stable
                )
                sensor = _sensor_with(adapter)
                before = list(sensor._gas_data)

                reading = sensor.read()

                self.assertEqual(reading["iaq"], 0)
                self.assertEqual(list(sensor._gas_data), before)

    def test_sensor_reads_are_serialized(self):
        adapter = _FakeReadingAdapter(delay=0.01)
        sensor = _sensor_with(adapter)

        with ThreadPoolExecutor(max_workers=8) as executor:
            readings = list(executor.map(lambda _index: sensor.read(), range(8)))

        self.assertTrue(all(reading is not None for reading in readings))
        self.assertEqual(adapter.max_active, 1)
        self.assertEqual(sum(value != 0 for value in sensor._gas_data), 8)


if __name__ == "__main__":
    unittest.main()
