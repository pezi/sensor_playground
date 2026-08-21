//! Sensor Playground Sensor Node — Chirp I2C Soil Moisture Sensor (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers with a Catnip Electronics I2C Soil Moisture Sensor (the
//! "Chirp" sensor, default I2C address 0x20). It reports the soil
//! moisture as a percentage (JSON key `moisture`, mapped linearly between
//! the two capacitance calibration points in config.json), the soil
//! temperature (`temperature`) and the ambient light level (`light`, raw
//! brightness counts, higher = brighter), alongside the raw capacitance
//! (`cap`) for calibrating.
//!
//! The chip measures light by timing a phototransistor discharge, which
//! takes up to three seconds — so the light value is harvested from a
//! measurement started on an earlier read, and the `light` key is absent
//! until the first one completes (a few seconds after start).
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

mod driver;

use common::{config, now_secs, wifi, Payload};
use driver::{decode_temperature, light_counts, moisture_percent};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_i2c_bus")]
    i2c_bus: u8,
    #[serde(default = "default_address")]
    address: u16,
    #[serde(default = "default_cap_dry")]
    cap_dry: i64,
    #[serde(default = "default_cap_wet")]
    cap_wet: i64,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
    #[serde(default = "default_ssl_cert")]
    ssl_cert: String,
    #[serde(default = "default_ssl_key")]
    ssl_key: String,
}

fn default_i2c_bus() -> u8 {
    1
}
fn default_address() -> u16 {
    0x20
}
fn default_cap_dry() -> i64 {
    290
}
fn default_cap_wet() -> i64 {
    520
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

/// The full-key REST reading, or None on a failed read.
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating CHIRP readings without hardware");
        // A watering cycle for the moisture, a steady room temperature
        // and a slow day/night curve for the light. Like the Python
        // node, the emulation ignores the configured calibration points
        // and uses the defaults.
        const CAP_DRY: i64 = 290;
        const CAP_WET: i64 = 520;
        return Box::new(|| {
            let t = now_secs();
            let cycle = (t % 300.0) / 300.0; // rewatered every five minutes
            let pct = 85.0 - 60.0 * cycle + 2.0 * (t / 3.0).sin();
            let capacitance =
                (CAP_DRY as f64 + (CAP_WET - CAP_DRY) as f64 * pct / 100.0).round() as i64;
            let raw_temp = (10.0 * (21.5 + 1.5 * (t / 60.0).sin())).round() as i16;
            let raw_light = (20000.0 + 15000.0 * (t / 120.0).sin()).round() as u16;
            let mut p = Payload::new();
            p.insert(
                "moisture".into(),
                json!(moisture_percent(capacitance, CAP_DRY, CAP_WET)),
            );
            p.insert(
                "temperature".into(),
                json!(decode_temperature(raw_temp as u16)),
            );
            p.insert("cap".into(), json!(capacitance));
            // The emulated measurement finishes instantly, so unlike the
            // real sensor the key is present from the first read.
            p.insert("light".into(), json!(light_counts(raw_light)));
            Some(p)
        });
    }
    if cfg.cap_dry == cfg.cap_wet {
        println!("Error: cap_dry and cap_wet must differ (calibrate!)");
        exit(1);
    }
    println!(
        "Initializing Chirp sensor on I2C bus {} (dry={}, wet={})...",
        cfg.i2c_bus, cfg.cap_dry, cfg.cap_wet
    );
    #[cfg(target_os = "linux")]
    {
        let (cap_dry, cap_wet) = (cfg.cap_dry, cfg.cap_wet);
        let mut dev = match driver::hw::Chirp::new(cfg.i2c_bus, cfg.address) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let r = match dev.read() {
                Ok(r) => r,
                Err(err) => {
                    println!("Sensor read failed: {err}");
                    return None;
                }
            };
            let mut p = Payload::new();
            p.insert(
                "moisture".into(),
                json!(moisture_percent(r.capacitance, cap_dry, cap_wet)),
            );
            p.insert("temperature".into(), json!(r.temperature));
            // The raw capacitance helps calibrate cap_dry/cap_wet.
            p.insert("cap".into(), json!(r.capacitance));
            if let Some(light) = r.light {
                p.insert("light".into(), json!(light));
            }
            Some(p)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real Chirp sensor requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

/// Short-key reading for the discovery reply.
fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(moisture), Some(temperature)) = (full.get("moisture"), full.get("temperature")) {
        p.insert("moist".into(), moisture.clone());
        p.insert("temp".into(), temperature.clone());
    }
    if let Some(light) = full.get("light") {
        p.insert("light".into(), light.clone());
    }
    p
}

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);

    let reader = Arc::new(Mutex::new(make_reader(&cfg)));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let host = hostname.clone();
            let reader = reader.clone();
            let build_payload = Box::new(move || {
                let data = (reader.lock().unwrap())()?;
                let mut p = Payload::new();
                p.insert("sensor".into(), json!("CHIRP"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                "CHIRP",
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
            // Identity-only reply on a failed read.
            let read_discovery: wifi::ReadFn = Box::new(move || {
                let full = (reader.lock().unwrap())();
                Some(full.map(|f| to_discovery(&f)).unwrap_or_default())
            });
            if let Err(err) = wifi::run_discovery_listener(
                "CHIRP",
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
        "CHIRP",
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
