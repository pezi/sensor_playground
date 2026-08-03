"""
Sensor Tester Sensor Node — BMP085 barometer (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a BMP085 I2C barometer — the sensor behind the
Grove Barometer Sensor, and the ancestor of the BME280 (no humidity, no gas).
https://wiki.seeedstudio.com/Grove-Barometer_Sensor/

Reports temperature, barometric pressure, and the altitude derived from it.

The driver is written out here rather than pulled from a library: the BMP085
returns *uncompensated* readings that only become values after Bosch's
fixed-point compensation, and the ESP32 sketch has to run the identical
arithmetic anyway. Keeping both copies next to each other — and testable
against the datasheet's worked example — is worth more than a dependency.
The pin-compatible BMP180 uses the same registers and works unchanged.

- HTTPS REST API on port 9132 + UDP discovery on port 9133 (default), or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import math
import random
import socket
import sys
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# -- Chip constants ----------------------------------------------------------

# The BMP085's I2C address is fixed — there is no address pin.
I2C_ADDRESS = 0x77

REG_CALIBRATION = 0xAA  # 22 bytes: AC1..AC6, B1, B2, MB, MC, MD
REG_CHIP_ID = 0xD0      # reads 0x55 on a BMP085 (and on a BMP180)
REG_CONTROL = 0xF4
REG_DATA = 0xF6

CMD_READ_TEMPERATURE = 0x2E
CMD_READ_PRESSURE = 0x34

CHIP_ID = 0x55

# Conversion time per oversampling setting, in seconds (datasheet table 3,
# rounded up for margin).
CONVERSION_TIME = {0: 0.005, 1: 0.008, 2: 0.014, 3: 0.026}

# Standard sea-level pressure, in pascal. Altitude is relative to this, so it
# moves with the weather as much as with the height.
SEA_LEVEL_PA = 101325.0


# -- Compensation (BMP085 datasheet, section 3.5) -----------------------------


class Calibration:
    """The eleven factory constants stored in the sensor's EEPROM."""

    def __init__(self, ac1, ac2, ac3, ac4, ac5, ac6, b1, b2, mb, mc, md):
        self.ac1 = ac1
        self.ac2 = ac2
        self.ac3 = ac3
        self.ac4 = ac4
        self.ac5 = ac5
        self.ac6 = ac6
        self.b1 = b1
        self.b2 = b2
        self.mb = mb
        self.mc = mc
        self.md = md

    @classmethod
    def from_bytes(cls, data):
        """Parses the 22 calibration bytes read from REG_CALIBRATION."""
        if len(data) != 22:
            raise ValueError(f"need 22 calibration bytes, got {len(data)}")

        # AC4, AC5 and AC6 are the only unsigned words.
        signedness = [True, True, True, False, False, False,
                      True, True, True, True, True]
        values = []
        for index, signed in enumerate(signedness):
            raw = (data[index * 2] << 8) | data[index * 2 + 1]
            # The datasheet states no calibration word is ever 0x0000 or
            # 0xFFFF, which is exactly what a bus with nothing (or nothing
            # powered) on it reads back. Catching it here beats compensating
            # with garbage — or dividing by zero a few lines further down.
            if raw in (0x0000, 0xFFFF):
                raise ValueError(
                    f"implausible calibration word {index} = 0x{raw:04X} "
                    "(bad I2C read?)"
                )
            values.append(raw - 0x10000 if signed and raw > 0x7FFF else raw)
        return cls(*values)


def _trunc_div(numerator, denominator):
    """Integer division that truncates toward zero, as C's `/` does.

    Python's `//` floors instead, which differs for a negative numerator —
    and the one division the datasheet writes as `/` rather than a shift has
    exactly that (MC is negative). Getting this wrong shifts B5 by one, i.e.
    0.1 °C and a little pressure, and would quietly put this node out of step
    with the ESP32 sketch.
    """
    quotient = abs(numerator) // abs(denominator)
    negative = (numerator < 0) != (denominator < 0)
    return -quotient if negative else quotient


def compensate(calibration, raw_temperature, raw_pressure, oversampling=0):
    """Turns raw readings into (temperature °C, pressure Pa).

    A direct transcription of the integer algorithm in the BMP085 datasheet.
    The shifts are arithmetic in both C and Python, so `>>` carries over as
    written; only the one true division needs [_trunc_div]. Floating point
    would be easier to read and would drift from the reference values the
    datasheet publishes.
    """
    c = calibration

    # Temperature.
    x1 = ((raw_temperature - c.ac6) * c.ac5) >> 15
    # `mc * 2048` rather than `mc << 11`: identical here, but MC is negative
    # and shifting a negative value left is undefined behaviour in the C the
    # ESP32 sketch mirrors. Keeping both spellings the same keeps them honest.
    x2 = _trunc_div(c.mc * 2048, x1 + c.md)
    b5 = x1 + x2
    temperature = ((b5 + 8) >> 4) / 10.0  # datasheet yields 0.1 °C steps

    # Pressure.
    b6 = b5 - 4000
    x1 = (c.b2 * ((b6 * b6) >> 12)) >> 11
    x2 = (c.ac2 * b6) >> 11
    x3 = x1 + x2
    b3 = (((c.ac1 * 4 + x3) << oversampling) + 2) >> 2
    x1 = (c.ac3 * b6) >> 13
    x2 = (c.b1 * ((b6 * b6) >> 12)) >> 16
    x3 = ((x1 + x2) + 2) >> 2
    b4 = (c.ac4 * (x3 + 32768)) >> 15
    b7 = (raw_pressure - b3) * (50000 >> oversampling)
    # B7 is unsigned 32-bit in the datasheet and can exceed 2^31, which is why
    # it is scaled before rather than after the division in that case.
    pressure = (b7 * 2) // b4 if b7 < 0x80000000 else (b7 // b4) * 2
    x1 = (pressure >> 8) * (pressure >> 8)
    x1 = (x1 * 3038) >> 16
    x2 = (-7357 * pressure) >> 16
    pressure += (x1 + x2 + 3791) >> 4

    return temperature, pressure


def altitude_for(pressure_pa, sea_level_pa=SEA_LEVEL_PA):
    """Altitude in metres from pressure, per the international barometric
    formula the BMP085 datasheet quotes."""
    return 44330.0 * (1.0 - (pressure_pa / sea_level_pa) ** (1.0 / 5.255))


# -- Sensor ------------------------------------------------------------------


class BMP085Sensor:
    """Reads temperature and pressure from a BMP085 (or BMP180) over I2C."""

    def __init__(self, i2c_bus=1, address=I2C_ADDRESS, oversampling=3):
        from smbus2 import SMBus

        if oversampling not in CONVERSION_TIME:
            raise ValueError(
                f"oversampling must be 0-3, got {oversampling!r}"
            )
        self.bus = SMBus(i2c_bus)
        self.address = address
        self.oversampling = oversampling
        self.name = "BMP085"

        chip_id = self.bus.read_byte_data(self.address, REG_CHIP_ID)
        if chip_id != CHIP_ID:
            raise RuntimeError(
                f"No BMP085 at address {address:#04x}: chip id is "
                f"{chip_id:#04x}, expected {CHIP_ID:#04x}."
            )
        self.calibration = Calibration.from_bytes(
            bytes(self.bus.read_i2c_block_data(self.address, REG_CALIBRATION, 22))
        )

    def _read_raw_temperature(self):
        self.bus.write_byte_data(self.address, REG_CONTROL, CMD_READ_TEMPERATURE)
        time.sleep(CONVERSION_TIME[0])  # temperature ignores oversampling
        data = self.bus.read_i2c_block_data(self.address, REG_DATA, 2)
        return (data[0] << 8) | data[1]

    def _read_raw_pressure(self):
        self.bus.write_byte_data(
            self.address,
            REG_CONTROL,
            CMD_READ_PRESSURE + (self.oversampling << 6),
        )
        time.sleep(CONVERSION_TIME[self.oversampling])
        data = self.bus.read_i2c_block_data(self.address, REG_DATA, 3)
        raw = (data[0] << 16) | (data[1] << 8) | data[2]
        return raw >> (8 - self.oversampling)

    def read(self):
        """Return full-key readings for the REST API."""
        # Temperature first, and every time: its B5 term feeds the pressure
        # compensation, so a stale one skews the pressure as the chip warms.
        temperature, pressure_pa = compensate(
            self.calibration,
            self._read_raw_temperature(),
            self._read_raw_pressure(),
            self.oversampling,
        )
        return {
            "temperature": round(temperature, 1),
            "pressure": round(pressure_pa / 100.0, 2),  # Pa -> hPa
            "altitude": round(altitude_for(pressure_pa), 1),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        return {
            "temp": full["temperature"],
            "press": full["pressure"],
            "alt": full["altitude"],
        }


class EmulatedBMP085Sensor(BMP085Sensor):
    """Generates plausible BMP085 readings without hardware.

    A room near sea level: temperature and pressure drift on slow sines of
    different periods around 21 °C / 1013 hPa, with a little measurement
    noise. The altitude is derived from the emulated pressure, so the three
    values stay consistent with each other.
    """

    def __init__(self):
        self.name = "BMP085"

    def read(self):
        t = time.time()
        temperature = 21.0 + 2.0 * math.sin(t / 60.0) + random.uniform(-0.1, 0.1)
        pressure_hpa = (
            1013.0 + 3.0 * math.sin(t / 300.0) + random.uniform(-0.2, 0.2)
        )
        return {
            "temperature": round(temperature, 1),
            "pressure": round(pressure_hpa, 2),
            "altitude": round(altitude_for(pressure_hpa * 100.0), 1),
        }


_sensor = None
_api_key = ""
_hostname = ""


# -- Main --------------------------------------------------------------------


def main():
    global _sensor, _api_key, _hostname

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    _hostname = config.get("hostname", "") or socket.gethostname()
    i2c_bus = config.get("i2c_bus", 1)
    oversampling = config.get("oversampling", 3)
    ssl_cert = config.get("ssl_cert", "cert.pem")
    ssl_key = config.get("ssl_key", "key.pem")

    if config.get("emulation", False):
        print("Emulation mode: generating BMP085 readings without hardware")
        _sensor = EmulatedBMP085Sensor()
    else:
        print(f"Initializing BMP085 sensor on /dev/i2c-{i2c_bus}...")
        _sensor = BMP085Sensor(i2c_bus=i2c_bus, oversampling=oversampling)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_poll(
            _sensor.name,
            _api_key,
            lambda: {"sensor": _sensor.name, "host": _hostname, **_sensor.read()},
        )
        return

    wifi.start_discovery_thread(
        _sensor.name, _hostname, wifi.HTTPS_PORT, _sensor.read_discovery
    )
    wifi.run_rest_server(
        _sensor.name, _api_key, _hostname, _sensor.read, ssl_cert, ssl_key
    )


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        print("Stopped.")
