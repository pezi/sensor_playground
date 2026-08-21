//! GPIO character-device relay bank for the 1-/2-channel modules.

#![cfg(target_os = "linux")]

use crate::relay::RelayBank;
use gpiocdev::line::Value;
use gpiocdev::Request;

struct OutputLine {
    request: Request,
    offset: u32,
}

impl OutputLine {
    fn open(chip: &str, offset: u32, active_low: bool) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-relay")
            .with_line(offset)
            .as_output(Value::Inactive);
        if active_low {
            builder.as_active_low();
        }
        let request = builder
            .request()
            .map_err(|err| format!("requesting relay GPIO {offset} on {chip}: {err}"))?;
        Ok(Self { request, offset })
    }

    fn set(&self, on: bool) -> Result<(), String> {
        self.request
            .set_value(
                self.offset,
                if on { Value::Active } else { Value::Inactive },
            )
            .map_err(|err| format!("writing relay GPIO {}: {err}", self.offset))
    }
}

pub struct GpioRelayBank {
    lines: Vec<OutputLine>,
}

impl GpioRelayBank {
    pub fn open(chip: &str, pins: &[u32], active_low: bool) -> Result<Self, String> {
        if pins.is_empty() || pins.len() > 8 {
            return Err("relay_pins must list 1..8 GPIO lines".into());
        }
        let lines = pins
            .iter()
            .map(|&pin| OutputLine::open(chip, pin, active_low))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { lines })
    }
}

impl RelayBank for GpioRelayBank {
    fn count(&self) -> usize {
        self.lines.len()
    }

    fn write(&mut self, states: &[bool]) -> Result<(), String> {
        if states.len() != self.lines.len() {
            return Err(format!(
                "got {} relay states for {} GPIO lines",
                states.len(),
                self.lines.len()
            ));
        }
        for (line, &on) in self.lines.iter().zip(states) {
            line.set(on)?;
        }
        Ok(())
    }
}
