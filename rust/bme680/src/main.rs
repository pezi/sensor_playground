//! Sensor Playground Sensor Node — BME680 (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a BME680 I2C sensor (temperature,
//! humidity, pressure, IAQ).
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

#[cfg(any(target_os = "linux", test))]
mod driver;
mod sensor;

use common::{config, wifi, Payload};
use sensor::{EmulatedBme680, Sensor, SensorData};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};

#[derive(Deserialize)]
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

fn full_payload(d: &SensorData) -> Payload {
    let mut p = Payload::new();
    p.insert("temperature".into(), json!(d.temperature));
    p.insert("humidity".into(), json!(d.humidity));
    p.insert("pressure".into(), json!(d.pressure));
    p.insert("iaq".into(), json!(d.iaq));
    p
}

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);

    let sensor: Box<dyn Sensor> = if cfg.emulation {
        println!("Emulation mode: generating BME680 readings without hardware");
        Box::new(EmulatedBme680::new())
    } else {
        println!("Initializing BME680 sensor on /dev/i2c-{}...", cfg.i2c_bus);
        #[cfg(target_os = "linux")]
        {
            match sensor::real::RealBme680::new(cfg.i2c_bus) {
                Ok(s) => Box::new(s),
                Err(err) => {
                    println!("Error: {err}");
                    exit(1);
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            println!("Error: the real BME680 requires Linux (set \"emulation\": true elsewhere)");
            exit(1);
        }
    };
    let sensor = Arc::new(Mutex::new(sensor));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let host = hostname.clone();
            let build_payload = Box::new(move || {
                let data = sensor.lock().unwrap().read()?;
                let mut p = Payload::new();
                p.insert("sensor".into(), json!("BME680"));
                p.insert("host".into(), json!(host));
                for (k, v) in full_payload(&data) {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                "BME680",
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
        let sensor = sensor.clone();
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            let read_discovery: wifi::ReadFn = Box::new(move || {
                let data = sensor.lock().unwrap().read();
                let mut p = Payload::new();
                if let Some(d) = data {
                    p.insert("temp".into(), json!(d.temperature));
                    p.insert("hum".into(), json!(d.humidity));
                    p.insert("press".into(), json!(d.pressure));
                    p.insert("iaq".into(), json!(d.iaq));
                }
                Some(p)
            });
            if let Err(err) = wifi::run_discovery_listener(
                "BME680",
                &hostname,
                wifi::HTTPS_PORT,
                Some(read_discovery),
            ) {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    let read: wifi::ReadFn = Box::new(move || {
        let data = sensor.lock().unwrap().read()?;
        Some(full_payload(&data))
    });
    if let Err(err) = wifi::run_rest_server(
        "BME680",
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
