//! GPIO access for the speaker pin via the character device
//! (/dev/gpiochipN, gpiocdev crate) — /sys/class/gpio is gone in Debian 13.

#![cfg(target_os = "linux")]

use gpiocdev::line::Value;
use gpiocdev::Request;

pub struct OutputLine {
    req: Request,
    offset: u32,
}

impl OutputLine {
    /// Request a line as output, starting low (silent).
    pub fn open(chip: &str, offset: u32, active_low: bool) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-speaker")
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
