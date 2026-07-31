"""
Sensor Tester Sensor Node — Grove IMU 10DOF (Python)

Implements the Sensor Tester Sensor Interface on single-board computers
(Raspberry Pi & co.) with a Grove IMU 10DOF board (MPU9250 + BMP280).
The MPU9250 axes are reduced to roll / pitch / compass heading angles plus
the total acceleration magnitude (g-force); the BMP280 adds temperature and
barometric pressure.

- WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
  (default) — the app streams motion sensors rather than polling them, or
- BLE GATT server ("transport": "ble" in config.json), like the ESP32 sketch

Set "emulation": true in config.json to generate plausible readings without
the sensor hardware (works with both transports).

Usage:
    cp config.example.json config.json   # edit with your settings
    python3 sensor_node.py
"""

import asyncio
import math
import random
import socket
import sys
import time
from pathlib import Path


# The shared transports (Wi-Fi + BLE) live in the sibling folder.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent / "common"))

import wifi_transport as wifi

# The app streams motion sensors instead of polling them, so readings are
# pushed at the same 250 ms cadence the BLE transport and the ESP32 use.
PUSH_INTERVAL = 0.25

# -- BMP280 barometer (register access via smbus2) ---------------------------


class BMP280:
    """Minimal BMP280 driver implementing the Bosch datasheet algorithm.

    The Grove IMU 10DOF v2.0 (2016) replaced the original BMP180 with a
    BMP280; the two chips share the I2C address but use different registers
    and a different compensation algorithm.
    """

    _CHIP_ID = 0x58  # value of the id register (0xD0) for the BMP280

    def __init__(self, bus, address=0x77):
        self.bus = bus
        self.address = address

        chip_id = self.bus.read_byte_data(self.address, 0xD0)
        if chip_id != self._CHIP_ID:
            raise RuntimeError(
                f"BMP280 not found at 0x{address:02X} "
                f"(id register returned 0x{chip_id:02X})"
            )

        calib = self.bus.read_i2c_block_data(self.address, 0x88, 24)

        def u16(index):
            return calib[index] | (calib[index + 1] << 8)

        def s16(index):
            value = u16(index)
            return value - 65536 if value > 32767 else value

        self.dig_t1 = u16(0)
        self.dig_t2, self.dig_t3 = s16(2), s16(4)
        self.dig_p1 = u16(6)
        self.dig_p2, self.dig_p3 = s16(8), s16(10)
        self.dig_p4, self.dig_p5 = s16(12), s16(14)
        self.dig_p6, self.dig_p7 = s16(16), s16(18)
        self.dig_p8, self.dig_p9 = s16(20), s16(22)

        # ctrl_meas: temperature x1, pressure x1, normal mode.
        self.bus.write_byte_data(self.address, 0xF4, 0x27)
        # config: 1000 ms standby, filter off.
        self.bus.write_byte_data(self.address, 0xF5, 0xA0)
        time.sleep(0.05)

    def read(self):
        """Return (temperature in °C, pressure in hPa)."""
        raw = self.bus.read_i2c_block_data(self.address, 0xF7, 6)
        adc_p = (raw[0] << 12) | (raw[1] << 4) | (raw[2] >> 4)
        adc_t = (raw[3] << 12) | (raw[4] << 4) | (raw[5] >> 4)

        # Temperature compensation (datasheet 32-bit integer algorithm).
        var1 = (((adc_t >> 3) - (self.dig_t1 << 1)) * self.dig_t2) >> 11
        var2 = (
            ((((adc_t >> 4) - self.dig_t1) * ((adc_t >> 4) - self.dig_t1)) >> 12)
            * self.dig_t3
        ) >> 14
        t_fine = var1 + var2
        temperature = ((t_fine * 5 + 128) >> 8) / 100.0

        # Pressure compensation (datasheet 64-bit integer algorithm).
        var1 = t_fine - 128000
        var2 = var1 * var1 * self.dig_p6
        var2 += (var1 * self.dig_p5) << 17
        var2 += self.dig_p4 << 35
        var1 = ((var1 * var1 * self.dig_p3) >> 8) + ((var1 * self.dig_p2) << 12)
        var1 = (((1 << 47) + var1) * self.dig_p1) >> 33
        if var1 == 0:
            return temperature, 0.0  # avoid division by zero
        p = 1048576 - adc_p
        p = (((p << 31) - var2) * 3125) // var1
        var1 = (self.dig_p9 * (p >> 13) * (p >> 13)) >> 25
        var2 = (self.dig_p8 * p) >> 19
        p = ((p + var1 + var2) >> 8) + (self.dig_p7 << 4)

        # p is in Q24.8 Pa; convert to hPa.
        return temperature, (p / 256.0) / 100.0


# -- Sensor ------------------------------------------------------------------


class IMU10DOFSensor:
    """Reads roll/pitch/heading/g-force plus temperature and pressure."""

    def __init__(self, i2c_bus=1):
        from mpu9250_jmdev.mpu_9250 import MPU9250
        from mpu9250_jmdev.registers import (
            AK8963_ADDRESS,
            AK8963_BIT_16,
            AK8963_MODE_C100HZ,
            AFS_8G,
            GFS_1000,
            MPU9050_ADDRESS_68,
        )
        from smbus2 import SMBus

        self.mpu = MPU9250(
            address_ak=AK8963_ADDRESS,
            address_mpu_master=MPU9050_ADDRESS_68,
            bus=i2c_bus,
            gfs=GFS_1000,
            afs=AFS_8G,
            mfs=AK8963_BIT_16,
            mode=AK8963_MODE_C100HZ,
        )
        self.mpu.configure()
        self.bmp = BMP280(SMBus(i2c_bus))
        self.name = "IMU10DOF"

    def read(self):
        """Return full-key readings for the live payload."""
        ax, ay, az = self.mpu.readAccelerometerMaster()
        mx, my, _ = self.mpu.readMagnetometerMaster()

        roll = math.degrees(math.atan2(ay, az))
        pitch = math.degrees(math.atan2(-ax, math.sqrt(ay * ay + az * az)))
        heading = math.degrees(math.atan2(my, mx)) % 360.0
        gforce = math.sqrt(ax * ax + ay * ay + az * az)

        temperature, pressure = self.bmp.read()
        return {
            "temperature": round(temperature, 1),
            "pressure": round(pressure, 2),
            "roll": round(roll, 1),
            "pitch": round(pitch, 1),
            "heading": round(heading, 1),
            "gforce": round(gforce, 2),
        }

    def read_discovery(self):
        """Return short-key readings for the UDP discovery response."""
        full = self.read()
        return {
            "temp": full["temperature"],
            "press": full["pressure"],
            "roll": full["roll"],
            "pitch": full["pitch"],
            "heading": full["heading"],
            "gforce": full["gforce"],
        }


class EmulatedIMU10DOFSensor(IMU10DOFSensor):
    """Generates plausible IMU 10DOF readings without hardware.

    A gently rocking, near-level board: roll and pitch follow slow sines
    with different periods, the g-force stays around 1 g and the heading
    sweeps a full circle every two minutes. The BMP280 side reports room
    temperature and sea-level pressure, each drifting slowly.
    """

    def __init__(self):
        self.name = "IMU10DOF"

    def read(self):
        t = time.time()
        return {
            "temperature": round(22.0 + 2.0 * math.sin(t / 60.0), 1),
            "pressure": round(1013.0 + 3.0 * math.sin(t / 300.0), 2),
            "roll": round(8.0 * math.sin(t / 7.0) + random.uniform(-0.3, 0.3), 1),
            "pitch": round(5.0 * math.sin(t / 11.0) + random.uniform(-0.3, 0.3), 1),
            "heading": round((t * 3) % 360, 1),
            "gforce": round(1.0 + random.uniform(-0.02, 0.02), 2),
        }


_sensor = None
_api_key = ""
_hostname = ""


async def push_loop(publish):
    """Push a reading every PUSH_INTERVAL, like the ESP32 sketch."""
    while True:
        data = _sensor.read()
        if data:
            await publish({"sensor": _sensor.name, "host": _hostname, **data})
        await asyncio.sleep(PUSH_INTERVAL)


async def main_async():
    server = wifi.WsPushServer(_api_key)
    async with server.serve():
        await push_loop(server.broadcast)


# -- Main --------------------------------------------------------------------


def main():
    global _sensor, _api_key, _hostname

    config = wifi.load_config(Path(__file__).parent)
    _api_key = config["api_key"]
    _hostname = config.get("hostname", "") or socket.gethostname()
    i2c_bus = config.get("i2c_bus", 1)

    if config.get("emulation", False):
        print("Emulation mode: generating IMU10DOF readings without hardware")
        _sensor = EmulatedIMU10DOFSensor()
    else:
        print(f"Initializing IMU 10DOF on /dev/i2c-{i2c_bus}...")
        _sensor = IMU10DOFSensor(i2c_bus=i2c_bus)

    if config.get("transport", "wifi") == "ble":
        import ble_transport

        ble_transport.run_ble_poll(
            _sensor.name,
            _api_key,
            lambda: {"sensor": _sensor.name, "host": _hostname, **_sensor.read()},
            interval=ble_transport.MOTION_NOTIFY_INTERVAL,
        )
        return

    wifi.start_discovery_thread(
        _sensor.name, _hostname, wifi.WS_PORT, _sensor.read_discovery
    )

    asyncio.run(main_async())


if __name__ == "__main__":
    main()
