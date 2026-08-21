//! Single-wire trigger/echo timing for the Grove Ultrasonic Ranger via
//! the GPIO character device (/dev/gpiochipN, gpiocdev crate). The SIG
//! line is requested once and then *reconfigured* in place for each
//! measurement — output for the trigger pulse, edge-event input for the
//! echo — the same mode flips the Python node does via lgpio claims. The
//! echo is timed from the kernel's edge timestamps: a userspace polling
//! loop would add milliseconds of jitter, and one millisecond of pulse
//! error is 17 cm of distance error.

#![cfg(target_os = "linux")]

use crate::{pulse_to_mm, ECHO_TIMEOUT, TRIGGER_PULSE};
use gpiocdev::line::{EdgeDetection, EdgeKind, Value};
use gpiocdev::request::Config;
use gpiocdev::Request;
use std::time::{Duration, Instant};

pub struct SigLine {
    req: Request,
    offset: u32,
    output_cfg: Config,
    input_cfg: Config,
}

impl SigLine {
    /// Request the SIG line (initially as output, idle low) and prepare
    /// the two per-measurement configurations.
    pub fn open(chip: &str, offset: u32) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-ultrasonic")
            .with_line(offset)
            .as_output(Value::Inactive);
        let req = builder
            .request()
            .map_err(|e| format!("requesting GPIO line {offset} on {chip}: {e}"))?;

        let mut output_cfg = Config::default();
        output_cfg.with_line(offset).as_output(Value::Inactive);
        let mut input_cfg = Config::default();
        input_cfg
            .with_line(offset)
            .as_input()
            .with_edge_detection(EdgeDetection::BothEdges);

        Ok(Self {
            req,
            offset,
            output_cfg,
            input_cfg,
        })
    }

    /// One measurement: trigger pulse out, then time the echo pulse.
    /// Returns the distance in mm, or None without an echo in range.
    pub fn read_mm(&self) -> Result<Option<i64>, String> {
        let err = |e: gpiocdev::Error| e.to_string();

        // Trigger: a >=10 µs high pulse on the line as output.
        self.req.reconfigure(&self.output_cfg).map_err(err)?;
        std::thread::sleep(TRIGGER_PULSE);
        self.req
            .set_value(self.offset, Value::Active)
            .map_err(err)?;
        std::thread::sleep(TRIGGER_PULSE);
        self.req
            .set_value(self.offset, Value::Inactive)
            .map_err(err)?;

        // Echo: hand the pin to the kernel's edge machinery and wait for
        // the pulse, discarding any stale event of a previous measurement.
        self.req.reconfigure(&self.input_cfg).map_err(err)?;
        while self.req.has_edge_event().map_err(err)? {
            self.req.read_edge_event().map_err(err)?;
        }

        let deadline = Instant::now() + ECHO_TIMEOUT;
        let mut rise_ns: Option<u64> = None;
        loop {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() || !self.req.wait_edge_event(remaining).map_err(err)? {
                return Ok(None); // no echo
            }
            let event = self.req.read_edge_event().map_err(err)?;
            match event.kind {
                EdgeKind::Rising => rise_ns = Some(event.timestamp_ns),
                EdgeKind::Falling => {
                    if let Some(rise) = rise_ns {
                        return Ok(pulse_to_mm(event.timestamp_ns.wrapping_sub(rise)));
                    }
                }
            }
        }
    }
}
