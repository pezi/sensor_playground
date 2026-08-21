//! GPIO access for the LED and button via the character device
//! (/dev/gpiochipN, gpiocdev crate) — /sys/class/gpio is gone in
//! Debian 13. Logical polarity: the kernel applies active-low translation,
//! so `set(true)` always means "light the LED" and a pressed button always
//! reads true.

#![cfg(target_os = "linux")]

use gpiocdev::line::{Bias, Value};
use gpiocdev::Request;

pub struct OutputLine {
    req: Request,
    offset: u32,
}

impl OutputLine {
    /// Request a line as output. With `active_low` the LED lights when the
    /// pin is driven LOW.
    pub fn open(chip: &str, offset: u32, active_low: bool) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-led")
            .with_line(offset)
            .as_output(Value::Inactive);
        if active_low {
            builder.as_active_low();
        }
        let req = builder
            .request()
            .map_err(|e| format!("requesting GPIO line {offset} on {chip}: {e}"))?;
        Ok(Self { req, offset })
    }

    pub fn set(&self, on: bool) {
        let _ = self.req.set_value(
            self.offset,
            if on { Value::Active } else { Value::Inactive },
        );
    }
}

pub struct InputLine {
    req: Request,
    offset: u32,
}

impl InputLine {
    /// Request a line as input. With `active_low` the line idles HIGH via
    /// the internal pull-up and reads true when pulled low (pressed);
    /// otherwise a pull-down and true on HIGH — matching gpiozero's
    /// pull_up semantics in the Python node.
    pub fn open(chip: &str, offset: u32, active_low: bool) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-button")
            .with_line(offset)
            .as_input();
        if active_low {
            builder.with_bias(Bias::PullUp).as_active_low();
        } else {
            builder.with_bias(Bias::PullDown);
        }
        let req = builder
            .request()
            .map_err(|e| format!("requesting GPIO line {offset} on {chip}: {e}"))?;
        Ok(Self { req, offset })
    }

    pub fn pressed(&self) -> bool {
        matches!(self.req.value(self.offset), Ok(Value::Active))
    }
}
