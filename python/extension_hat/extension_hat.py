"""
Extension hat helper for Sensor Tester nodes (Python / Raspberry Pi & co.)

A port of the dart_periphery `extension_hat.dart` driver to smbus2, so the
Python sensor nodes can drive Grove/BakeBit expansion hats over I2C:

* `NanoHatHub`     — FriendlyARM NanoHat Hub (BakeBit firmware, Arduino Nano)
                     http://wiki.friendlyelec.com/wiki/index.php/BakeBit_-_NanoHat_Hub
* `GrovePiPlusHat` — Seeed GrovePi+ (ATMEGA328P)
                     https://wiki.seeedstudio.com/GrovePi_Plus
* `GroveBaseHat`   — Seeed Grove Base Hat for Raspberry Pi / Pi Zero (STM32)
                     https://www.seeedstudio.com/Grove-Base-Hat-for-Raspberry-Pi.html

The first two are *Arduino-based*: the Pi sends a 4-byte command
(`[command, pin, value, value2]`) to register 1 of I2C address 0x04 and reads
the reply back from the same register. The Grove Base Hat is a pure ADC hat
with its own 16-bit register map.

Reference firmware:
https://github.com/friendlyarm/BakeBit/blob/master/Firmware/Source/bakebit_v1.0.0/bakebit_v1.0.0.ino

Usage:
    from extension_hat import NanoHatHub, GroveBaseHat, BakeBitLedBar

    hat = GroveBaseHat()           # I2C bus 1 by default
    print(hat.get_name(), hat.read_adc_raw(0))

Requires smbus2 (see requirements.txt) and an enabled I2C bus
(`sudo raspi-config` > Interface Options > I2C).
"""

import time
from enum import Enum, IntEnum

from smbus2 import SMBus

# -- Protocol constants ------------------------------------------------------

# Arduino-based hats answer on this I2C address; commands go to register 1.
HAT_I2C_ADDRESS = 0x04
HAT_REGISTER = 1

# Minimum spacing between two I2C actions, in milliseconds.
DELAY_MS = 50

# The original GrovePi+ firmware needs retries due to a hardware quirk; the
# NanoHat Hub is more reliable but the retries do no harm.
RETRY = 3

# Grove Base Hat product ids (see GroveBaseHat.get_id).
RPI_HAT_PID = 0x04
RPI_ZERO_HAT_PID = 0x05


# -- Enums -------------------------------------------------------------------


class HatType(Enum):
    """Supported extension hats."""

    NANO = "nano"
    GROVE_PLUS = "grovePlus"
    GROVE = "grove"


class Command(IntEnum):
    """Arduino-based hat command opcodes (first byte of a command)."""

    DIGITAL_READ = 1
    DIGITAL_WRITE = 2
    ANALOG_READ = 3
    ANALOG_WRITE = 4
    PIN_MODE = 5
    ULTRASONIC = 7
    FIRMWARE = 8
    LEDBAR_INIT = 110
    LEDBAR_RELEASE = 111
    LEDBAR_SHOW = 112
    SERVO_ATTACH = 120
    SERVO_DETACH = 121
    SERVO_WRITE = 122


class DigitalValue(IntEnum):
    """Digital pin level."""

    LOW = 0
    HIGH = 1

    def invert(self):
        """Returns the opposite level."""
        return DigitalValue.HIGH if self is DigitalValue.LOW else DigitalValue.LOW


class PinMode(IntEnum):
    """Pin direction."""

    INPUT = 0
    OUTPUT = 1


class HatCmd:
    """Builds the 4-byte command buffer for an Arduino-based hat.

    Format: byte 0 command, byte 1 pin, bytes 2-3 optional parameters.
    """

    def __init__(self, command):
        self.command = command

    def seq(self):
        """Returns a command buffer carrying only the command opcode."""
        return [self.command.value, 0, 0, 0]

    def seq_ext(self, pin, value=0, value2=0):
        """Returns a command buffer with a [pin] and up to two parameters."""
        return [self.command.value, pin, value, value2]


# -- Arduino-based hats (NanoHat Hub, GrovePi+) ------------------------------


class ArduinoBasedHat:
    """I2C link between the Pi and an Arduino-based hat (NanoHat / GrovePi+)."""

    def __init__(self, i2c_bus):
        self._bus = SMBus(i2c_bus)
        self._auto_wait = False
        self._last_action = 0.0

    def close(self):
        """Closes the underlying I2C bus."""
        self._bus.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    # -- auto-wait -----------------------------------------------------------

    def set_auto_wait(self, flag):
        """Enables or disables the automatic inter-command delay."""
        self._auto_wait = flag

    def _update_last_action(self):
        if self._auto_wait:
            self._last_action = time.monotonic()

    def _wait(self):
        """Ensures at least [DELAY_MS] ms have elapsed since the last action."""
        if not self._auto_wait:
            return
        elapsed_ms = (time.monotonic() - self._last_action) * 1000
        remaining_ms = DELAY_MS - elapsed_ms
        if remaining_ms > 0:
            time.sleep(remaining_ms / 1000)

    # -- raw block transfers (with retry) ------------------------------------

    def read_i2c_block(self, length):
        """Reads [length] bytes from the hat's command register."""
        self._wait()
        last_error = None
        for _ in range(RETRY):
            try:
                data = self._bus.read_i2c_block_data(
                    HAT_I2C_ADDRESS, HAT_REGISTER, length
                )
                self._update_last_action()
                return data
            except OSError as error:
                last_error = error
                time.sleep(DELAY_MS / 1000)
        raise last_error

    def write_i2c_block(self, data):
        """Writes a byte list to the hat's command register."""
        self._wait()
        last_error = None
        for _ in range(RETRY):
            try:
                self._bus.write_i2c_block_data(
                    HAT_I2C_ADDRESS, HAT_REGISTER, list(data)
                )
                self._update_last_action()
                return
            except OSError as error:
                last_error = error
                time.sleep(DELAY_MS / 1000)
        raise last_error

    # -- GPIO ----------------------------------------------------------------

    def pin_mode(self, pin, mode):
        """Sets the [mode] (input/output) of [pin]."""
        self.write_i2c_block(HatCmd(Command.PIN_MODE).seq_ext(pin, mode.value))

    def digital_read(self, pin):
        """Reads the [DigitalValue] of [pin]."""
        last_error = None
        for _ in range(RETRY):
            try:
                self.write_i2c_block(HatCmd(Command.DIGITAL_READ).seq_ext(pin))
                time.sleep(DELAY_MS / 1000)
                value = self._bus.read_byte_data(HAT_I2C_ADDRESS, HAT_REGISTER)
                self._update_last_action()
                return DigitalValue.HIGH if value else DigitalValue.LOW
            except OSError as error:
                last_error = error
                time.sleep(DELAY_MS / 1000)
        raise last_error

    def digital_write(self, pin, value):
        """Writes the digital [value] to [pin]."""
        self.write_i2c_block(
            HatCmd(Command.DIGITAL_WRITE).seq_ext(pin, value.value)
        )

    def analog_read(self, pin):
        """Reads the analog value of [pin]; returns -1 on an out-of-range read."""
        last_error = None
        for _ in range(RETRY):
            try:
                self.write_i2c_block(HatCmd(Command.ANALOG_READ).seq_ext(pin))
                time.sleep(DELAY_MS / 1000)
                data = self.read_i2c_block(4)
                value = (data[1] & 0xFF) << 8 | (data[2] & 0xFF)
                self._update_last_action()
                return -1 if value == 65535 else value
            except OSError as error:
                last_error = error
                time.sleep(DELAY_MS / 1000)
        raise last_error

    def analog_write(self, pin, value):
        """Writes the analog [value] (PWM duty) to [pin]."""
        self.write_i2c_block(
            HatCmd(Command.ANALOG_WRITE).seq_ext(pin, value)
        )

    def get_firmware_version(self):
        """Returns the hat firmware version as a `major.minor.patch` string."""
        last_error = None
        for _ in range(RETRY):
            try:
                self.write_i2c_block(HatCmd(Command.FIRMWARE).seq())
                time.sleep(0.1)
                data = self.read_i2c_block(4)
                self._update_last_action()
                return f"{data[1] & 0xFF}.{data[2] & 0xFF}.{data[3] & 0xFF}"
            except OSError as error:
                last_error = error
                time.sleep(DELAY_MS / 1000)
        raise last_error

    def _send_cmd(self, command, pin, value1=0, value2=0):
        self.write_i2c_block(HatCmd(command).seq_ext(pin, value1, value2))


class NanoHatHub(ArduinoBasedHat):
    """FriendlyARM NanoHat Hub (BakeBit firmware).

    https://github.com/friendlyarm/BakeBit
    """

    def __init__(self, i2c_bus=0):
        super().__init__(i2c_bus)

    # -- LED bar -------------------------------------------------------------

    def led_bar_init_ext(self, pin, chipset, led_number):
        """Initializes the LED bar on [pin] for [chipset] and [led_number]."""
        self._send_cmd(Command.LEDBAR_INIT, pin, chipset, led_number)

    def led_bar_init(self, pin):
        """Initializes a 5-LED BakeBit LED bar on [pin]."""
        self._send_cmd(Command.LEDBAR_INIT, pin, 0, 5)

    def led_bar_show(self, pin, high_bits, low_bits):
        """Shows the LED bar on [pin] from a 16-bit mask (see [BakeBitLedBar])."""
        self._send_cmd(Command.LEDBAR_SHOW, pin, high_bits, low_bits)

    def led_bar_release(self, pin):
        """Releases the LED bar on [pin]."""
        self._send_cmd(Command.LEDBAR_RELEASE, pin)

    # -- servo ---------------------------------------------------------------

    def servo_attach(self, pin):
        """Attaches the servo on [pin]."""
        self._send_cmd(Command.SERVO_ATTACH, pin)

    def servo_detach(self, pin):
        """Detaches the servo on [pin]."""
        self._send_cmd(Command.SERVO_DETACH, pin)

    def servo_write(self, pin, position):
        """Steers the servo on [pin] to [position] (0-180 degrees)."""
        self._send_cmd(Command.SERVO_WRITE, pin, position)

    # -- ultrasonic ranger ---------------------------------------------------

    def read_ultrasonic(self, pin):
        """Reads the ultrasonic ranger on [pin], in cm (range ~5-300)."""
        last_error = None
        for _ in range(RETRY):
            try:
                self.write_i2c_block(HatCmd(Command.ULTRASONIC).seq_ext(pin))
                time.sleep(0.1)
                data = self.read_i2c_block(3)
                self._update_last_action()
                return (data[1] & 0xFF) * 256 | (data[2] & 0xFF)
            except OSError as error:
                last_error = error
                time.sleep(DELAY_MS / 1000)
        raise last_error


class GrovePiPlusHat(ArduinoBasedHat):
    """Seeed GrovePi+ hat (ATMEGA328P).

    https://wiki.seeedstudio.com/GrovePi_Plus/

    Not recommended: its UART misbehaves with some devices (e.g. the CozIR
    CO2 sensor) and it struggles with more than two I2C devices or I2C+SPI at
    once. Kept for parity with the dart_periphery driver.
    """

    def __init__(self, i2c_bus=1):
        super().__init__(i2c_bus)


# -- BakeBit LED bar helper --------------------------------------------------


class LedBarColor(IntEnum):
    """LED bar color (see [NanoHatHub.led_bar_init_ext])."""

    GREEN = 0
    RED = 1
    YELLOW = 2
    BLUE = 3
    GHOST_WHITE = 4
    ORANGE = 5
    CYAN = 6


class LedBarLed(IntEnum):
    """LED bar position (see [NanoHatHub.led_bar_init_ext])."""

    LED1 = 0
    LED2 = 1
    LED3 = 2
    LED4 = 3
    LED5 = 4


class BakeBitLedBar:
    """Builds the 16-bit color mask for a BakeBit LED bar.

    Each of the five LEDs gets a 3-bit color code; feed [high_bits] and
    [low_bits] to [NanoHatHub.led_bar_show].
    """

    def __init__(self):
        self.bit_mask = 0

    def set_led(self, led, color):
        """Sets [led] (LED1-LED5) to [color]."""
        self.bit_mask |= (color.value + 1) << (led.value * 3)

    def low_bits(self):
        """Returns the lower 8 bits of the mask."""
        return self.bit_mask & 0xFF

    def high_bits(self):
        """Returns the upper 8 bits of the mask."""
        return (self.bit_mask & 0xFF00) >> 8

    def get_bit_mask(self):
        """Returns the full 16-bit mask."""
        return self.bit_mask


# -- Grove Base Hat (STM32 ADC hat) ------------------------------------------


class GroveBaseHat:
    """Seeed Grove Base Hat for Raspberry Pi / Pi Zero (STM32F030 ADC hat).

    https://wiki.seeedstudio.com/Grove_Base_Hat_for_Raspberry_Pi/

    A 12-bit ADC reachable over a 16-bit register map at I2C address 0x04 —
    unlike the Arduino-based hats it speaks no command protocol.
    """

    def __init__(self, i2c_bus=1):
        self._bus = SMBus(i2c_bus)
        self._id = 0

    def close(self):
        """Closes the underlying I2C bus."""
        self._bus.close()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()

    def _read_16bit_register(self, command):
        return self._bus.read_word_data(HAT_I2C_ADDRESS, command)

    @staticmethod
    def _check_channel(channel):
        if not 0 <= channel <= 7:
            raise ValueError(f"Invalid channel {channel} - valid range [0,7]")

    def get_id(self):
        """Returns the hat product id (cached): 0x04 RPi, 0x05 RPi Zero."""
        if self._id == 0:
            self._id = self._read_16bit_register(0x00)
        return self._id

    def get_name(self):
        """Returns the human-readable hat model name."""
        return {
            RPI_HAT_PID: "Grove Base Hat RPi",
            RPI_ZERO_HAT_PID: "Grove Base Hat RPi Zero",
        }.get(self.get_id(), "Unknown Hat model")

    def is_hat_rpi_zero(self):
        """Returns True for the Pi Zero model, False for the Pi model."""
        return self.get_id() == RPI_ZERO_HAT_PID

    def get_firmware(self):
        """Returns the firmware version."""
        return self._read_16bit_register(0x02)

    def read_adc_raw(self, channel):
        """Reads the raw 12-bit ADC value [0-4095] of [channel] (0-7)."""
        self._check_channel(channel)
        return self._read_16bit_register(0x10 + channel)

    def read_input_voltage(self, channel):
        """Reads the input voltage of [channel] (0-7), in mV."""
        self._check_channel(channel)
        return self._read_16bit_register(0x20 + channel)

    def get_grove_power_supply_voltage(self):
        """Returns the hat supply voltage, in mV."""
        return self._read_16bit_register(0x29)

    def read_ratio(self, channel):
        """Returns the input/supply voltage ratio of [channel] (0-7), in 0.1%."""
        self._check_channel(channel)
        return self._read_16bit_register(0x30 + channel)

    def change_i2c_address(self, new_address):
        """Changes the hat's I2C address. NOT TESTED."""
        value = (0xC0 << 16) | (new_address & 0xFF)
        self._bus.write_i2c_block_data(
            HAT_I2C_ADDRESS, 0, [value & 0xFF, (value >> 8) & 0xFF, (value >> 16) & 0xFF]
        )

    def reset_hat(self):
        """Resets the hat. NOT TESTED."""
        self._bus.write_byte_data(HAT_I2C_ADDRESS, 0, 0xF0)
