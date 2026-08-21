//! Low-pulse-occupancy accumulation for the Grove Dust Sensor
//! (PPD42NS) via the GPIO character device (/dev/gpiochipN, gpiocdev
//! crate). The sensor pulls the line LOW for roughly 10-90 ms per
//! particle event, so both edges are timestamped by the kernel: a
//! userspace polling loop would add milliseconds of jitter per edge —
//! a large error on a 10 ms pulse.
//!
//! A watcher thread drains the kernel's edge events into the shared
//! accumulator; read() lazily closes the 30-second window once it is
//! due (the same lazy roll the Python node does).

#![cfg(target_os = "linux")]

use crate::{window_concentration, WINDOW_S};
use gpiocdev::line::{EdgeDetection, EdgeKind, Value};
use gpiocdev::Request;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The clock the kernel stamps GPIO edge events with
/// (CLOCK_MONOTONIC), so a pulse spanning a window boundary can be
/// split in the events' own time base.
fn kernel_now_ns() -> u64 {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: clock_gettime only writes the timespec we hand it.
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return 0;
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

struct State {
    lpo_ns: u64,       // nanoseconds of low time this window
    low_since_ns: u64, // kernel timestamp of the current falling edge
    in_low: bool,
    window_start: Instant,
    result: Option<f64>, // None until the first window closes
}

pub struct Ppd42ns {
    state: Arc<Mutex<State>>,
}

impl Ppd42ns {
    /// Request the pin as an edge-event input and start the watcher.
    /// The PPD42NS drives its output actively (through the divider),
    /// so no internal bias is set.
    pub fn open(chip: &str, offset: u32) -> Result<Self, String> {
        let mut builder = Request::builder();
        builder
            .on_chip(chip)
            .with_consumer("sensor-playground-ppd42ns")
            .with_line(offset)
            .as_input()
            .with_edge_detection(EdgeDetection::BothEdges);
        let req = builder
            .request()
            .map_err(|e| format!("requesting GPIO line {offset} on {chip}: {e}"))?;

        let state = Arc::new(Mutex::new(State {
            lpo_ns: 0,
            low_since_ns: 0,
            in_low: false,
            window_start: Instant::now(),
            result: None,
        }));

        // Edge events never fire for a pin that is already low (a stuck
        // or misbehaving line), which would read as 0 % occupancy —
        // "perfectly clean air" — instead of saturation. Treat an
        // initial low as a pulse in progress so a stuck-low line
        // reports a huge value, not a clean one.
        if matches!(req.value(offset), Ok(Value::Inactive)) {
            let mut s = state.lock().unwrap();
            s.in_low = true;
            s.low_since_ns = kernel_now_ns();
        }

        let watched = state.clone();
        std::thread::spawn(move || watch(req, watched));
        Ok(Self { state })
    }

    /// Closes the LPO window once it is due and caches the result.
    ///
    /// Called lazily from read(), so the window grows past WINDOW_S
    /// when nobody polls — harmless, because the ratio divides by the
    /// *actual* elapsed time rather than the nominal window length.
    fn maybe_roll(&mut self) {
        let now = Instant::now();
        let mut s = self.state.lock().unwrap();
        let elapsed = now.duration_since(s.window_start).as_secs_f64();
        if elapsed < WINDOW_S {
            return;
        }
        let mut lpo_ns = s.lpo_ns;
        if s.in_low {
            // A pulse spans the boundary: credit the elapsed part to
            // the closing window and restart the timer for the new one.
            let kernel_now = kernel_now_ns();
            if kernel_now > s.low_since_ns {
                lpo_ns += kernel_now - s.low_since_ns;
            }
            s.low_since_ns = kernel_now;
        }
        s.lpo_ns = 0;
        s.window_start = now;
        s.result = Some(window_concentration(lpo_ns, elapsed));
    }

    /// The dust concentration [pcs/0.01cf], or None while warming up.
    pub fn read(&mut self) -> Option<f64> {
        self.maybe_roll();
        self.state.lock().unwrap().result
    }
}

/// Accumulates low time from the kernel's edge timestamps: a falling
/// edge starts a pulse, a rising edge credits it to the current window.
fn watch(req: Request, state: Arc<Mutex<State>>) {
    loop {
        match req.wait_edge_event(Duration::from_secs(1)) {
            Ok(false) => continue, // timeout — no edge within a second
            Ok(true) => {}
            Err(err) => {
                println!("GPIO event wait failed: {err}");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        }
        let event = match req.read_edge_event() {
            Ok(event) => event,
            Err(err) => {
                println!("GPIO event read failed: {err}");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        let mut s = state.lock().unwrap();
        match event.kind {
            EdgeKind::Rising => {
                if s.in_low && event.timestamp_ns > s.low_since_ns {
                    s.lpo_ns += event.timestamp_ns - s.low_since_ns;
                }
                s.in_low = false;
            }
            EdgeKind::Falling => {
                s.low_since_ns = event.timestamp_ns;
                s.in_low = true;
            }
        }
    }
}
