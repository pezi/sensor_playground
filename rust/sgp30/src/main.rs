//! Sensor Playground Sensor Node — SGP30 (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a SGP30 I2C sensor (eCO2, TVOC).
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

use common::{config, uniform, wifi, Payload};
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
fn default_transport() -> String {
    "wifi".into()
}
fn default_ssl_cert() -> String {
    "cert.pem".into()
}
fn default_ssl_key() -> String {
    "key.pem".into()
}

/// The full-key REST reading, or None on a failed read. Emulation: indoor
/// air quality drifting around typical office values — eCO2 does a bounded
/// random walk between 400 and 1500 ppm, TVOC between 0 and 600 ppb (like
/// the Python node).
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating SGP30 readings without hardware");
        let mut eco2 = 600.0f64;
        let mut tvoc = 60.0f64;
        return Box::new(move || {
            eco2 = (eco2 + uniform(-15.0, 15.0)).clamp(400.0, 1500.0);
            tvoc = (tvoc + uniform(-8.0, 8.0)).clamp(0.0, 600.0);
            let mut p = Payload::new();
            p.insert("eco2".into(), json!(eco2.round() as i64));
            p.insert("tvoc".into(), json!(tvoc.round() as i64));
            Some(p)
        });
    }
    println!("Initializing SGP30 sensor on /dev/i2c-{}...", cfg.i2c_bus);
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Sgp30::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        println!("Warming up SGP30 (about 15 seconds)...");
        if let Err(err) = dev.start_measurement() {
            println!("Error: {err}");
            exit(1);
        }
        Box::new(move || {
            let (eco2, tvoc) = dev.read().ok()?;
            let mut p = Payload::new();
            p.insert("eco2".into(), json!(eco2));
            p.insert("tvoc".into(), json!(tvoc));
            Some(p)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real SGP30 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
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
            let build_payload = Box::new(move || {
                let data = (reader.lock().unwrap())()?;
                let mut p = Payload::new();
                p.insert("sensor".into(), json!("SGP30"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) =
                common::ble::run_ble("SGP30", cfg.api_key, common::ble::BleRole::poll(build_payload))
            {
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
            // The Python node's read_discovery returns the same full keys.
            let read_discovery: wifi::ReadFn = Box::new(move || {
                let full = (reader.lock().unwrap())();
                Some(full.unwrap_or_default())
            });
            if let Err(err) = wifi::run_discovery_listener(
                "SGP30",
                &hostname,
                wifi::HTTPS_PORT,
                Some(read_discovery),
            ) {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    let read: wifi::ReadFn = Box::new(move || (reader.lock().unwrap())());
    if let Err(err) =
        wifi::run_rest_server("SGP30", cfg.api_key, hostname, read, &cfg.ssl_cert, &cfg.ssl_key)
    {
        println!("Error: {err}");
        exit(1);
    }
}
