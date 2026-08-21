//! GPIO access for the contact input via the character device
//! (/dev/gpiochipN, gpiocdev crate) — /sys/class/gpio is gone in
//! Debian 13. Logical polarity: the kernel applies the active-low
//! translation, so a triggered sensor always reads true.

#![cfg(target_os = "linux")]

use gpiocdev::line::{Bias, Value};
use gpiocdev::Request;

pub struct InputLine {
    req: Request,
    offset: u32,
}

impl InputLine {
    /// Request a line as input. With `active_low` the line idles HIGH via
    /// the internal pull-up and reads true when pulled low (triggered);
    /// otherwise a pull-down and true on HIGH — matching gpiozero's
    /// pull_up semantics in the Python node. The pull-down matters for
    /// reed-based sensors like the Grove Magnetic Switch, whose released
    /// state floats.
    pub fn open(chip: &str, offset: u32, active_low: bool) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-contact")
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

    /// Returns true while the sensor is triggered.
    pub fn active(&self) -> bool {
        matches!(self.req.value(self.offset), Ok(Value::Active))
    }
}
