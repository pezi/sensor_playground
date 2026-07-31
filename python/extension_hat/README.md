# Extension Hat Helper (Python)

A reusable helper for driving **Grove / BakeBit expansion hats** from the
Python sensor nodes on a Raspberry Pi (and similar SoCs). It is a port of the
dart_periphery
[`extension_hat.dart`](https://github.com/pezi/dart_periphery/blob/main/lib/src/hardware/extension_hat.dart)
driver onto `smbus2`, the same I2C library the SSD1306 node uses.

Unlike the other folders here this is **not a sensor node** — it has no UDP
discovery or WebSocket/REST server. It is a library you import from a node (or
your own script) when the sensor you want to expose is wired through a hat
rather than directly to the Pi's I2C bus.

## Supported hats

| Class | Hardware | I2C bus (default) |
|-------|----------|-------------------|
| `NanoHatHub` | [FriendlyARM NanoHat Hub](http://wiki.friendlyelec.com/wiki/index.php/BakeBit_-_NanoHat_Hub) (BakeBit firmware, Arduino Nano) | `0` |
| `GrovePiPlusHat` | [Seeed GrovePi+](https://wiki.seeedstudio.com/GrovePi_Plus/) (ATMEGA328P) | `1` |
| `GroveBaseHat` | [Seeed Grove Base Hat for Pi / Pi Zero](https://wiki.seeedstudio.com/Grove_Base_Hat_for_Raspberry_Pi/) (STM32, 12-bit ADC) | `1` |

There are two protocol families:

- **Arduino-based hats** (`NanoHatHub`, `GrovePiPlusHat`): the Pi sends a
  4-byte command `[command, pin, value, value2]` to register 1 of I2C address
  `0x04` and reads the reply from the same register. This gives digital I/O,
  analog read/write (PWM), firmware version, plus — on the NanoHat — an LED
  bar, servo control and an ultrasonic ranger.
- **Grove Base Hat** (`GroveBaseHat`): a pure ADC hat with a 16-bit register
  map. No command protocol — just `read_adc_raw` / `read_input_voltage` on
  channels 0-7.

> **Heads-up:** GrovePi+ is kept only for parity. Its UART misbehaves with
> some devices (e.g. the CozIR CO2 sensor) and it has trouble with more than
> two I2C devices or with I2C + SPI at once. Prefer the Grove Base Hat or the
> NanoHat Hub.

## Setup

```bash
python3 -m venv venv
source venv/bin/activate
pip install -r requirements.txt
```

Enable I2C on the Pi (`sudo raspi-config` > Interface Options > I2C), then
confirm the hat is present at address `0x04`:

```bash
i2cdetect -y 1      # bus 1 for the Grove Base Hat / GrovePi+
i2cdetect -y 0      # bus 0 for the NanoHat Hub
```

## Usage

### Grove Base Hat — read an analog sensor

```python
from extension_hat import GroveBaseHat

with GroveBaseHat() as hat:               # I2C bus 1
    print(hat.get_name())                 # "Grove Base Hat RPi"
    raw = hat.read_adc_raw(0)             # channel A0, 0-4095
    millivolts = hat.read_input_voltage(0)
    print(raw, millivolts, "mV")
```

### NanoHat Hub — LED bar, servo, ultrasonic, GPIO

```python
from extension_hat import (
    NanoHatHub, BakeBitLedBar, LedBarLed, LedBarColor,
    PinMode, DigitalValue,
)

with NanoHatHub() as hat:                 # I2C bus 0
    hat.set_auto_wait(True)               # auto-space commands by 50 ms
    print(hat.get_firmware_version())

    bar = BakeBitLedBar()
    bar.set_led(LedBarLed.LED1, LedBarColor.GREEN)
    bar.set_led(LedBarLed.LED5, LedBarColor.RED)
    hat.led_bar_init(5)                   # LED bar on port D5
    hat.led_bar_show(5, bar.high_bits(), bar.low_bits())

    print(hat.read_ultrasonic(3), "cm")   # ranger on port D3

    hat.pin_mode(6, PinMode.OUTPUT)
    hat.digital_write(6, DigitalValue.HIGH)
```

See [`example.py`](example.py) for a runnable version.

## API summary

**`ArduinoBasedHat`** (base of `NanoHatHub` / `GrovePiPlusHat`)
`pin_mode`, `digital_read`, `digital_write`, `analog_read`, `analog_write`,
`get_firmware_version`, `set_auto_wait`, `close`.

**`NanoHatHub`** adds
`led_bar_init`, `led_bar_init_ext`, `led_bar_show`, `led_bar_release`,
`servo_attach`, `servo_detach`, `servo_write`, `read_ultrasonic`.

**`GroveBaseHat`**
`get_id`, `get_name`, `is_hat_rpi_zero`, `get_firmware`, `read_adc_raw`,
`read_input_voltage`, `get_grove_power_supply_voltage`, `read_ratio`,
`change_i2c_address` *(untested)*, `reset_hat` *(untested)*, `close`.

**`BakeBitLedBar`** builds the 16-bit color mask for `led_bar_show`:
`set_led`, `high_bits`, `low_bits`, `get_bit_mask`.

## Notes

- All I2C calls retry up to 3 times with a 50 ms pause, matching the
  dart_periphery driver (the GrovePi+ firmware needs this).
- `set_auto_wait(True)` guarantees at least 50 ms between actions on the
  Arduino-based hats — useful when issuing commands in a tight loop.
