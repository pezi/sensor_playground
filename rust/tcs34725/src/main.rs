//! Sensor Playground Sensor Node — TCS34725 (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a TCS34725 I2C sensor (RGB colour,
//! colour temperature, illuminance).
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

use common::{config, now_secs, uniform, wifi, Payload};
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

/// The full-key REST reading, or None on a failed or saturated read.
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn payload(red: u8, green: u8, blue: u8, lux: f64, color_temperature: f64) -> Payload {
    let mut p = Payload::new();
    p.insert(
        "colorTemperature".into(),
        json!(color_temperature.round() as i64),
    );
    p.insert("lux".into(), json!(lux.round() as i64));
    p.insert("red".into(), json!(red));
    p.insert("green".into(), json!(green));
    p.insert("blue".into(), json!(blue));
    p
}

/// Convert a hue/saturation/value triple to red, green and blue in 0..1 —
/// the same sextant maths as Python's colorsys.hsv_to_rgb, used only to
/// drive the emulated colour cycle.
fn hsv_to_rgb(h: f64, s: f64, v: f64) -> (f64, f64, f64) {
    if s == 0.0 {
        return (v, v, v);
    }
    let sextant = (h * 6.0) as i64;
    let f = h * 6.0 - sextant as f64;
    let p = v * (1.0 - s);
    let q = v * (1.0 - s * f);
    let t = v * (1.0 - s * (1.0 - f));
    match sextant % 6 {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    }
}

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating TCS34725 readings without hardware");
        // A coloured light slowly cycling through the hue circle every 30
        // seconds, with the colour temperature swinging between warm and
        // cool white and the illuminance drifting around a few hundred lux
        // (like the Python node).
        return Box::new(|| {
            let t = now_secs();
            let (r, g, b) = hsv_to_rgb((t / 30.0).rem_euclid(1.0), 0.6, 0.9);
            let byte = |x: f64| (x * 255.0).round() as u8;
            Some(payload(
                byte(r),
                byte(g),
                byte(b),
                300.0 + 200.0 * (t / 75.0).sin() + uniform(-5.0, 5.0),
                4600.0 + 1900.0 * (t / 45.0).sin(),
            ))
        });
    }
    println!(
        "Initializing TCS34725 sensor on /dev/i2c-{}...",
        cfg.i2c_bus
    );
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Tcs34725::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            // A None reading means the clear channel saturated: there is no
            // colour to report.
            let (color, lux, color_temperature) = dev.read().ok()??;
            Some(payload(
                color.red,
                color.green,
                color.blue,
                lux,
                color_temperature,
            ))
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real TCS34725 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(ct), Some(lux)) = (full.get("colorTemperature"), full.get("lux")) {
        p.insert("ct".into(), ct.clone());
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
                p.insert("sensor".into(), json!("TCS34725"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                "TCS34725",
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
                "TCS34725",
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
        "TCS34725",
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
