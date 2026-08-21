//! Sensor Playground Sensor Node — BMP085 barometer (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a BMP085 I2C barometer — the sensor
//! behind the Grove Barometer Sensor. Reports temperature, barometric
//! pressure, and the altitude derived from it. The pin-compatible BMP180
//! works unchanged.
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

use common::{config, now_secs, round1, round2, uniform, wifi, Payload};
use driver::altitude_for;
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
    #[serde(default = "default_oversampling")]
    oversampling: u8,
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
fn default_oversampling() -> u8 {
    3
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

type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating BMP085 readings without hardware");
        // A room near sea level, drifting on slow sines; the altitude is
        // derived from the emulated pressure so the values stay consistent.
        return Box::new(|| {
            let t = now_secs();
            let pressure_hpa = 1013.0 + 3.0 * (t / 300.0).sin() + uniform(-0.2, 0.2);
            let mut p = Payload::new();
            p.insert("temperature".into(), json!(round1(21.0 + 2.0 * (t / 60.0).sin() + uniform(-0.1, 0.1))));
            p.insert("pressure".into(), json!(round2(pressure_hpa)));
            p.insert("altitude".into(), json!(round1(altitude_for(pressure_hpa * 100.0))));
            Some(p)
        });
    }
    println!("Initializing BMP085 sensor on /dev/i2c-{}...", cfg.i2c_bus);
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Bmp085::new(cfg.i2c_bus, cfg.oversampling) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let (temperature, pressure_pa) = dev.read().ok()?;
            let mut p = Payload::new();
            p.insert("temperature".into(), json!(round1(temperature)));
            p.insert("pressure".into(), json!(round2(pressure_pa as f64 / 100.0))); // Pa -> hPa
            p.insert("altitude".into(), json!(round1(altitude_for(pressure_pa as f64))));
            Some(p)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real BMP085 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(t), Some(pr), Some(a)) = (
        full.get("temperature"),
        full.get("pressure"),
        full.get("altitude"),
    ) {
        p.insert("temp".into(), t.clone());
        p.insert("press".into(), pr.clone());
        p.insert("alt".into(), a.clone());
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
                p.insert("sensor".into(), json!("BMP085"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) =
                common::ble::run_ble("BMP085", cfg.api_key, common::ble::BleRole::poll(build_payload))
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
                "BMP085",
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
        wifi::run_rest_server("BMP085", cfg.api_key, hostname, read, &cfg.ssl_cert, &cfg.ssl_key)
    {
        println!("Error: {err}");
        exit(1);
    }
}
