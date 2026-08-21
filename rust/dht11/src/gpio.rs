//! Single-wire DHT11/DHT22 framing via the GPIO character device
//! (/dev/gpiochipN, gpiocdev crate). The SIG line is requested once and
//! then *reconfigured* in place for each read — output for the 18 ms
//! start signal, falling-edge input for the sensor's answer — so no
//! second request has to win a race against the first data edge. The
//! bits are told apart by the kernel's edge timestamps: a 0 slot is
//! ~76-78 µs falling-to-falling, a 1 slot ~120 µs, a difference a
//! userspace polling loop would drown in jitter.

#![cfg(target_os = "linux")]

use crate::{FALLING_EDGES, READ_TIMEOUT, START_SIGNAL};
use gpiocdev::line::{EdgeDetection, Value};
use gpiocdev::request::Config;
use gpiocdev::Request;
use std::time::{Duration, Instant};

pub struct DhtLine {
    req: Request,
    output_cfg: Config,
    input_cfg: Config,
}

impl DhtLine {
    /// Request the SIG line and prepare the two per-read configurations.
    /// The idle state is edge-input: the line stays released (the
    /// module's pull-up holds it high), which is what the sensor expects
    /// between frames.
    pub fn open(chip: &str, offset: u32) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-dht11")
            .with_line(offset)
            .as_input()
            .with_edge_detection(EdgeDetection::FallingEdge);
        let req = builder
            .request()
            .map_err(|e| format!("requesting GPIO line {offset} on {chip}: {e}"))?;

        let mut output_cfg = Config::default();
        output_cfg.with_line(offset).as_output(Value::Inactive);
        let mut input_cfg = Config::default();
        input_cfg
            .with_line(offset)
            .as_input()
            .with_edge_detection(EdgeDetection::FallingEdge);

        Ok(Self {
            req,
            output_cfg,
            input_cfg,
        })
    }

    /// One single-wire read: start signal out, then collect the
    /// falling-edge timestamps of the answer. Returns what arrived before
    /// the timeout — `decode_frame` decides whether that is a whole frame.
    pub fn sample(&self) -> Result<Vec<u64>, String> {
        let err = |e: gpiocdev::Error| e.to_string();

        // Drop stale events left over from the previous frame. Safe to do
        // here: the line is in edge-input mode between reads and the
        // sensor stays quiet until it is asked again.
        while self.req.has_edge_event().map_err(err)? {
            self.req.read_edge_event().map_err(err)?;
        }

        // Start signal: as output the line is driven low (edge detection
        // is off in this configuration, so this pull generates no event).
        self.req.reconfigure(&self.output_cfg).map_err(err)?;
        std::thread::sleep(START_SIGNAL);

        // Release the line — the pull-up raises it and the sensor answers
        // within ~40 µs with the response preamble, then 40 bit slots.
        self.req.reconfigure(&self.input_cfg).map_err(err)?;

        let deadline = Instant::now() + READ_TIMEOUT;
        let mut falling = Vec::with_capacity(FALLING_EDGES);
        while falling.len() < FALLING_EDGES {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() || !self.req.wait_edge_event(remaining).map_err(err)? {
                break; // quiet line: decode what arrived
            }
            falling.push(self.req.read_edge_event().map_err(err)?.timestamp_ns);
        }
        Ok(falling)
    }
}
