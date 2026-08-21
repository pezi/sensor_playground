//! Sensor Playground Sensor Node — Grove Light Sensor (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers with a Grove Light Sensor — an analog photo-resistor
//! reporting a raw brightness value (higher = brighter) under the JSON
//! key `light`.
//!
//! The Raspberry Pi has no analog input, so the sensor is read through
//! the Seeed Grove Base Hat's 12-bit ADC (I2C address 0x04, one 16-bit
//! register per channel). The Arduino-based hats the Python node also
//! supports ("nano", "grovePlus") are not implemented in this port.
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

use common::{config, now_secs, uniform, wifi, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
const HAT_I2C_ADDRESS: u16 = 0x04; // Grove Base Hat (STM32F030 ADC)
#[allow(dead_code)]
const HAT_ADC_BASE: u8 = 0x10; // raw 12-bit value registers, one per channel

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_hat_type")]
    hat_type: String,
    #[serde(default)]
    pin: u8,
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

fn default_hat_type() -> String {
    "grove".into()
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

type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating LIGHT readings without hardware");
        // Daylight through a window: a slow sine around a few hundred
        // counts with a little flicker, never below zero.
        return Box::new(|| {
            let t = now_secs();
            let raw = 400.0 + 350.0 * (t / 120.0).sin() + uniform(-15.0, 15.0);
            let mut p = Payload::new();
            p.insert("light".into(), json!(raw.round().max(0.0) as i64));
            Some(p)
        });
    }
    if cfg.hat_type != "grove" {
        println!(
            "Error: hat_type \"{}\" is not supported in the Rust port (only \"grove\"; use the Python node for Arduino-based hats)",
            cfg.hat_type
        );
        exit(1);
    }
    if cfg.pin > 7 {
        println!("Error: invalid channel {} - valid range [0,7]", cfg.pin);
        exit(1);
    }
    println!(
        "Initializing Grove Light Sensor on grove hat, channel {}...",
        cfg.pin
    );
    #[cfg(target_os = "linux")]
    {
        let mut dev = match common::i2c::I2CDevice::open(cfg.i2c_bus, HAT_I2C_ADDRESS) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: opening /dev/i2c-{}: {err}", cfg.i2c_bus);
                exit(1);
            }
        };
        let reg = HAT_ADC_BASE + cfg.pin;
        Box::new(move || {
            // The raw 12-bit ADC value [0-4095] of the channel.
            let value = dev.read_word(reg).ok()?;
            let mut p = Payload::new();
            p.insert("light".into(), json!(value));
            Some(p)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the Grove Base Hat requires Linux (set \"emulation\": true elsewhere)");
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
                p.insert("sensor".into(), json!("LIGHT"));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) =
                common::ble::run_ble("LIGHT", cfg.api_key, common::ble::BleRole::poll(build_payload))
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
            // The light value uses the same key on both payloads.
            let read_discovery: wifi::ReadFn =
                Box::new(move || Some((reader.lock().unwrap())().unwrap_or_default()));
            if let Err(err) = wifi::run_discovery_listener(
                "LIGHT",
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
        wifi::run_rest_server("LIGHT", cfg.api_key, hostname, read, &cfg.ssl_cert, &cfg.ssl_key)
    {
        println!("Error: {err}");
        exit(1);
    }
}
