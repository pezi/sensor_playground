"""
Example usage of the extension_hat helper.

Pick the block that matches your hardware. Each is self-contained — these are
illustrations, not a sensor node, so there is no discovery/WebSocket plumbing.
"""

import time

from extension_hat import (
    BakeBitLedBar,
    DigitalValue,
    GroveBaseHat,
    LedBarColor,
    LedBarLed,
    NanoHatHub,
    PinMode,
)


def grove_base_hat_demo():
    """Read the ADC channels of a Seeed Grove Base Hat (Pi / Pi Zero)."""
    with GroveBaseHat() as hat:  # I2C bus 1
        print(f"Model: {hat.get_name()} (firmware {hat.get_firmware()})")
        print(f"Supply: {hat.get_grove_power_supply_voltage()} mV")
        for channel in range(8):
            raw = hat.read_adc_raw(channel)
            millivolts = hat.read_input_voltage(channel)
            print(f"  A{channel}: raw={raw:4d}  {millivolts} mV")


def nano_hat_demo():
    """Drive a FriendlyARM NanoHat Hub: LED bar, ultrasonic, a digital pin."""
    with NanoHatHub() as hat:  # I2C bus 0
        hat.set_auto_wait(True)
        print(f"Firmware: {hat.get_firmware_version()}")

        # Light the 5-LED bar on port D5 in a gradient.
        bar = BakeBitLedBar()
        bar.set_led(LedBarLed.LED1, LedBarColor.GREEN)
        bar.set_led(LedBarLed.LED2, LedBarColor.CYAN)
        bar.set_led(LedBarLed.LED3, LedBarColor.YELLOW)
        bar.set_led(LedBarLed.LED4, LedBarColor.ORANGE)
        bar.set_led(LedBarLed.LED5, LedBarColor.RED)
        hat.led_bar_init(5)
        hat.led_bar_show(5, bar.high_bits(), bar.low_bits())

        # Read an ultrasonic ranger on port D3.
        print(f"Distance: {hat.read_ultrasonic(3)} cm")

        # Blink an LED wired to digital port D6.
        hat.pin_mode(6, PinMode.OUTPUT)
        for _ in range(5):
            hat.digital_write(6, DigitalValue.HIGH)
            time.sleep(0.5)
            hat.digital_write(6, DigitalValue.LOW)
            time.sleep(0.5)


if __name__ == "__main__":
    grove_base_hat_demo()
    # nano_hat_demo()
