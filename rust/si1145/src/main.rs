//! Sensor Playground Sensor Node — SI1145 (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a SI1145 I2C sensor (visible light,
//! infrared, UV index).
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

use common::{config, now_secs, round1, uniform, wifi, Payload};
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

/// The full-key REST reading, or None on a failed read.
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn payload(visible: u16, ir: u16, uv: f64) -> Payload {
    let mut p = Payload::new();
    p.insert("visible".into(), json!(visible));
    p.insert("ir".into(), json!(ir));
    p.insert("uv".into(), json!(round1(uv)));
    p
}

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating SI1145 readings without hardware");
        // Indoor light near a window: visible and IR counts hover around
        // the chip's dark baseline (~260) with slow sines and a little
        // flicker, while the UV index sweeps between 0 and 8 over ten
        // minutes (like the Python node).
        return Box::new(|| {
            let t = now_secs();
            let counts = |x: f64| x.round() as u16;
            Some(payload(
                counts(262.0 + 40.0 * (t / 90.0).sin() + uniform(-3.0, 3.0)),
                counts(253.0 + 30.0 * (t / 70.0).sin() + uniform(-3.0, 3.0)),
                4.0 + 4.0 * (t / 600.0).sin(),
            ))
        });
    }
    println!("Initializing SI1145 sensor on /dev/i2c-{}...", cfg.i2c_bus);
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Si1145::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let (visible, ir, uv) = dev.read().ok()?;
            Some(payload(visible, ir, uv))
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real SI1145 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(visible), Some(ir), Some(uv)) =
        (full.get("visible"), full.get("ir"), full.get("uv"))
    {
        p.insert("vis".into(), visible.clone());
        p.insert("ir".into(), ir.clone());
        p.insert("uv".into(), uv.clone());
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
                p.insert("sensor".into(), json!("SI1145"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                "SI1145",
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
                "SI1145",
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
        "SI1145",
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
