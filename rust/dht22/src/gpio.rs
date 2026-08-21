//! DHT22 single-wire framing via the Linux GPIO character device. The line
//! is reconfigured in place for each sample: output for the start signal,
//! then falling-edge input for the sensor response.

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
    pub fn open(chip: &str, offset: u32) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-dht22")
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

    pub fn sample(&self) -> Result<Vec<u64>, String> {
        let err = |e: gpiocdev::Error| e.to_string();

        while self.req.has_edge_event().map_err(err)? {
            self.req.read_edge_event().map_err(err)?;
        }

        self.req.reconfigure(&self.output_cfg).map_err(err)?;
        std::thread::sleep(START_SIGNAL);
        self.req.reconfigure(&self.input_cfg).map_err(err)?;

        let deadline = Instant::now() + READ_TIMEOUT;
        let mut falling = Vec::with_capacity(FALLING_EDGES);
        while falling.len() < FALLING_EDGES {
            let remaining = deadline
                .checked_duration_since(Instant::now())
                .unwrap_or(Duration::ZERO);
            if remaining.is_zero() || !self.req.wait_edge_event(remaining).map_err(err)? {
                break;
            }
            falling.push(self.req.read_edge_event().map_err(err)?.timestamp_ns);
        }
        Ok(falling)
    }
}
