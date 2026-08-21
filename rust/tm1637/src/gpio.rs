//! The TM1637's two-wire bus, bit-banged on GPIO character-device lines
//! (/dev/gpiochipN, gpiocdev crate) — /sys/class/gpio is gone in
//! Debian 13. The chip speaks a proprietary protocol (start/stop
//! conditions like I2C, but LSB-first and without addresses) and has no
//! minimum clock speed, so one ioctl per transition paces the bus well
//! below the chip's limit.

#![cfg(target_os = "linux")]

use crate::driver::{segments_for_time, Display};
use gpiocdev::line::Value;
use gpiocdev::Request;

/// One line held as a plain push-pull output.
struct OutputLine {
    req: Request,
    offset: u32,
}

impl OutputLine {
    fn open(chip: &str, offset: u32) -> Result<Self, String> {
        let req = Request::builder()
            .on_chip(chip)
            .with_consumer("sensor-playground-tm1637")
            .with_line(offset)
            .as_output(Value::Inactive)
            .request()
            .map_err(|e| format!("requesting GPIO line {offset} on {chip}: {e}"))?;
        Ok(Self { req, offset })
    }

    fn set(&self, high: bool) {
        let _ = self.req.set_value(
            self.offset,
            if high { Value::Active } else { Value::Inactive },
        );
    }
}

/// Bit-bangs a TM1637 4-digit display on two GPIO lines.
pub struct GpioDisplay {
    clk: OutputLine,
    dio: OutputLine,
}

impl GpioDisplay {
    pub fn open(chip: &str, clk_pin: u32, dio_pin: u32) -> Result<Self, String> {
        Ok(Self {
            clk: OutputLine::open(chip, clk_pin)?,
            dio: OutputLine::open(chip, dio_pin)?,
        })
    }

    fn start(&self) {
        // DIO falls while CLK is high.
        self.clk.set(true);
        self.dio.set(true);
        self.dio.set(false);
        self.clk.set(false);
    }

    fn stop(&self) {
        // DIO rises while CLK is high.
        self.clk.set(false);
        self.dio.set(false);
        self.clk.set(true);
        self.dio.set(true);
    }

    fn write_byte(&self, value: u8) {
        for bit in 0..8 {
            self.clk.set(false);
            self.dio.set((value >> bit) & 1 == 1);
            self.clk.set(true);
        }
        // ACK slot: the chip acknowledges by pulling DIO low itself. Like
        // the Python node, drive DIO low through the ninth clock pulse
        // instead of re-requesting the line as input — the same level the
        // chip is asserting, so nothing conflicts, and there is nothing
        // useful to do on a NAK anyway.
        self.clk.set(false);
        self.dio.set(false);
        self.clk.set(true);
        self.clk.set(false);
    }
}

impl Display for GpioDisplay {
    fn render(&mut self, time: Option<(u8, u8)>, colon_on: bool, brightness: u8) {
        let segments = segments_for_time(time, colon_on);

        self.start();
        self.write_byte(0x40); // data command: write, auto-increment address
        self.stop();

        self.start();
        self.write_byte(0xC0); // address command: start at digit 0
        for segment in segments {
            self.write_byte(segment);
        }
        self.stop();

        self.start();
        self.write_byte(0x88 | (brightness & 0x07)); // display on
        self.stop();
    }

    fn close(&mut self) {
        // Blank the panel rather than leaving a stale time burning.
        self.render(None, false, 0);
    }
}
