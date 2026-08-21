//! Sensor Playground Sensor Node — SparkFun ISL29125 RGB Light Sensor (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers with a SparkFun RGB Light Sensor breakout — an
//! Intersil/Renesas ISL29125 (I2C address 0x44) measuring the intensity of
//! red, green and blue light while rejecting infrared. The channels are
//! normalized against the brightest one so the app can show the measured
//! colour directly (JSON keys red/green/blue, 0-255, absent in complete
//! darkness); an approximate illuminance (lux) is derived from the green
//! channel, whose spectral response resembles the human eye. There is no
//! clear channel, so unlike the TCS34725 no colour temperature is reported.
//! https://www.sparkfun.com/sparkfun-rgb-light-sensor-isl29125.html
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
use driver::derive_reading;
use serde::Deserialize;
#[cfg(target_os = "linux")]
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
    0x44
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
        println!("Emulation mode: generating ISL29125 readings without hardware");
        // Indoor light slowly shifting between warm and cool white, with
        // the brightness breathing over a couple of minutes (like the
        // Python node).
        return Box::new(|| {
            let t = now_secs();
            let brightness = 0.35 + 0.3 * (t / 120.0).sin();
            let warmth = 0.5 + 0.5 * (t / 45.0).sin();
            let counts = |x: f64| (65535.0 * x).round() as u16;
            Some(derive_reading(
                counts(brightness),
                counts(brightness * (0.6 + 0.4 * warmth)),
                counts(brightness * (1.0 - 0.5 * warmth)),
            ))
        });
    }
    println!(
        "Initializing ISL29125 on I2C bus {}, address {:#04x}...",
        cfg.i2c_bus, cfg.address
    );
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Isl29125::new(cfg.i2c_bus, cfg.address) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let (green, red, blue) = dev.read_channels().ok()?;
            Some(derive_reading(green, red, blue))
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real ISL29125 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

/// The discovery reply carries only the illuminance, as in the Python node.
fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let Some(lux) = full.get("lux") {
        p.insert("lux".into(), lux.clone());
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
            let build_payload = Box::new(move || {
                let data = (reader.lock().unwrap())()?;
                let mut p = Payload::new();
                p.insert("sensor".into(), json!("ISL29125"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                "ISL29125",
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
            let read_discovery: wifi::ReadFn = Box::new(move || {
                let full = (reader.lock().unwrap())();
                Some(full.map(|f| to_discovery(&f)).unwrap_or_default())
            });
            if let Err(err) = wifi::run_discovery_listener(
                "ISL29125",
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
        "ISL29125",
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
