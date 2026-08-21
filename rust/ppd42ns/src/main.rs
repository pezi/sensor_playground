//! Sensor Playground Sensor Node — Grove Dust Sensor / Shinyei PPD42NS (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a Grove Dust Sensor (Shinyei
//! PPD42NS). The sensor pulls its output pin LOW while particles scatter
//! light inside its chamber; the node accumulates that low-pulse
//! occupancy (LPO) over 30-second windows and converts the ratio into a
//! particle concentration in pcs/0.01cf using the Nafis curve, reported
//! under the JSON key `dust`:
//!
//!     ratio         = low_time / window_time * 100          (percent)
//!     concentration = 1.1*r^3 - 3.8*r^2 + 520*r + 0.62      (pcs/0.01cf)
//!
//! https://wiki.seeedstudio.com/Grove-Dust_Sensor/
//! https://www.howmuchsnow.com/arduino/airquality/grovedust/
//!
//! The pin is read directly via kernel-timestamped GPIO edge events
//! (gpiocdev) — there is no extension-hat option: the Arduino-based hats
//! are polled over I2C and cannot timestamp the sensor's 10-90 ms pulses.
//!
//! The first reading appears after the first full 30-second window; until
//! then the REST endpoint answers 503 and the discovery reply carries no
//! `dust` value. That is warm-up, not an error.
//!
//! - HTTPS REST API on port 9132 + UDP discovery on port 9133 (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only)
//!
//! Set "emulation": true in config.json to generate plausible readings
//! without the sensor hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod gpio;

use common::{config, now_secs, round1, uniform, wifi, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};

// -- LPO window math ----------------------------------------------------------

/// Length of one low-pulse-occupancy accumulation window.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const WINDOW_S: f64 = 30.0;

/// Converts a low-pulse-occupancy ratio [%] into a particle
/// concentration [pcs/0.01cf] — the Nafis curve.
fn concentration(ratio: f64) -> f64 {
    1.1 * ratio.powi(3) - 3.8 * ratio.powi(2) + 520.0 * ratio + 0.62
}

/// Closes one LPO window: `lpo_ns` nanoseconds of low time over
/// `elapsed_s` seconds of window time yield the concentration.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn window_concentration(lpo_ns: u64, elapsed_s: f64) -> f64 {
    let ratio = lpo_ns as f64 / 1e9 / elapsed_s * 100.0;
    concentration(ratio)
}

fn dust_payload(dust: f64) -> Payload {
    let mut p = Payload::new();
    p.insert("dust".into(), json!(round1(dust)));
    p
}

// -- Config -------------------------------------------------------------------

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_pin")]
    pin: u32,
    #[serde(default)]
    gpio_chip: u32,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
    #[serde(default = "default_ssl_cert")]
    ssl_cert: String,
    #[serde(default = "default_ssl_key")]
    ssl_key: String,
}

fn default_pin() -> u32 {
    17
}
fn default_transport() -> String {
    "wifi".into()
}
fn default_ssl_cert() -> String {
    "cert.pem".into()
}
fn default_ssl_key() -> String {
    "key.pem".into()
}

// -- Reader -------------------------------------------------------------------

/// The full-key REST reading, or None while warming up (first window).
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating PPD42NS readings without hardware");
        // Indoor air over the day: the concentration follows a slow sine
        // between roughly 100 and 700 pcs/0.01cf — deliberately crossing
        // several Dylos air-quality bands — with a little measurement
        // jitter, never below zero (like the Python node).
        return Box::new(|| {
            let t = now_secs();
            let c = 400.0 + 300.0 * (t / 180.0).sin() + uniform(-40.0, 40.0);
            Some(dust_payload(c.max(0.0)))
        });
    }
    println!(
        "Initializing Grove Dust Sensor (PPD42NS) on GPIO {}...",
        cfg.pin
    );
    println!("First reading after the first full 30-second LPO window.");
    #[cfg(target_os = "linux")]
    {
        let chip = format!("/dev/gpiochip{}", cfg.gpio_chip);
        let mut sensor = match gpio::Ppd42ns::open(&chip, cfg.pin) {
            Ok(sensor) => sensor,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || sensor.read().map(dust_payload))
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: GPIO requires Linux (use \"emulation\": true elsewhere)");
        exit(1);
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);

    let reader = Arc::new(Mutex::new(make_reader(&cfg)));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let host = hostname.clone();
            // Like the Python node, the BLE payload always carries the
            // identity; the dust value joins it after the first window.
            let build_payload = Box::new(move || {
                let mut p = Payload::new();
                p.insert("sensor".into(), json!("PPD42NS"));
                p.insert("host".into(), json!(host));
                if let Some(data) = (reader.lock().unwrap())() {
                    for (k, v) in data {
                        p.insert(k, v);
                    }
                }
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                "PPD42NS",
                cfg.api_key,
                common::ble::BleRole::poll(build_payload),
            ) {
                println!("Error: {err}");
                exit(1);
            }
            return;
        }
        #[cfg(not(target_os = "linux"))]
        {
            println!("Warning: BLE transport requires Linux, using wifi.");
        }
    }

    {
        let reader = reader.clone();
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            // The dust value uses the same key on both payloads; during
            // warm-up the reply falls back to identity-only (never None,
            // which would drop the whole reply).
            let read_discovery: wifi::ReadFn =
                Box::new(move || Some((reader.lock().unwrap())().unwrap_or_default()));
            if let Err(err) = wifi::run_discovery_listener(
                "PPD42NS",
                &hostname,
                wifi::HTTPS_PORT,
                Some(read_discovery),
            ) {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    let read: wifi::ReadFn = Box::new(move || (reader.lock().unwrap())());
    if let Err(err) = wifi::run_rest_server(
        "PPD42NS",
        cfg.api_key,
        hostname,
        read,
        &cfg.ssl_cert,
        &cfg.ssl_key,
    ) {
        println!("Error: {err}");
        exit(1);
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The ratio -> concentration conversion (Nafis curve). Golden values
    /// from the Python node's formula: 1.1*r^3 - 3.8*r^2 + 520*r + 0.62.
    #[test]
    fn concentration_curve_matches_python_formula() {
        let cases = [
            (0.0, 0.62),         // clean window
            (1.0, 517.92),       // 1 % occupancy
            (2.0, 1034.22),      // 2 % occupancy
            (5.5, 2928.6825),    // mid-range, exercises the cubic term
            (100.0, 1114000.62), // fully low window (stuck-low saturation)
        ];
        for (ratio, want) in cases {
            let got = concentration(ratio);
            assert!(
                (got - want).abs() < 1e-6,
                "concentration({ratio}) = {got}, want {want}"
            );
        }
    }

    /// Closing a window divides the low time by the *actual* elapsed
    /// time: 3 s of low time over 30 s is a 10 % ratio.
    #[test]
    fn window_concentration_uses_elapsed_time() {
        // ratio 0 %: only the curve's offset remains.
        assert!((window_concentration(0, 30.0) - 0.62).abs() < 1e-9);
        // ratio 10 %: 1.1*1000 - 3.8*100 + 520*10 + 0.62 = 5920.62.
        assert!((window_concentration(3_000_000_000, 30.0) - 5920.62).abs() < 1e-6);
        // A window that grew past its nominal length halves the ratio.
        assert!((window_concentration(3_000_000_000, 60.0) - concentration(5.0)).abs() < 1e-6);
    }
}
