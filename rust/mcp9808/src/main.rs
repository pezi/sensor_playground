//! Sensor Playground Sensor Node — MCP9808 (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a MCP9808 I2C sensor (temperature).
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

use common::{config, now_secs, round2, uniform, wifi, Payload};
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

/// The full-key REST reading, or None on a failed read. Emulation: a room
/// temperature drifting on a slow sine around 22 °C (like the Python node).
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating MCP9808 readings without hardware");
        return Box::new(|| {
            let t = now_secs();
            let mut p = Payload::new();
            p.insert("temperature".into(), json!(round2(22.0 + 2.0 * (t / 60.0).sin() + uniform(-0.1, 0.1))));
            Some(p)
        });
    }
    println!("Initializing MCP9808 sensor on /dev/i2c-{}...", cfg.i2c_bus);
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Mcp9808::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let temperature = dev.read().ok()?;
            let mut p = Payload::new();
            p.insert("temperature".into(), json!(round2(temperature)));
            Some(p)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real MCP9808 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let Some(t) = full.get("temperature") {
        p.insert("temp".into(), t.clone());
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
                p.insert("sensor".into(), json!("MCP9808"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) =
                common::ble::run_ble("MCP9808", cfg.api_key, common::ble::BleRole::poll(build_payload))
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
            let read_discovery: wifi::ReadFn = Box::new(move || {
                let full = (reader.lock().unwrap())();
                Some(full.map(|f| to_discovery(&f)).unwrap_or_default())
            });
            if let Err(err) = wifi::run_discovery_listener(
                "MCP9808",
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
        wifi::run_rest_server("MCP9808", cfg.api_key, hostname, read, &cfg.ssl_cert, &cfg.ssl_key)
    {
        println!("Error: {err}");
        exit(1);
    }
}
