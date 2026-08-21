//! Sensor Playground Sensor Node — SHT11 / Sensirion SHT1x (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a Sensirion SHT1x sensor
//! (temperature, humidity) — the classic SHT10 / SHT11 / SHT15 family.
//! The chips differ only in calibration accuracy and speak the same
//! proprietary two-wire protocol (SCK + bidirectional DATA); it resembles
//! I2C but is NOT I2C — the sensor cannot share an I2C bus. Set
//! "sensor_name" in config.json to the chip on your board so the app
//! shows the right name.
//!
//! The sensor must not be measured more than ~10% of the time or it heats
//! itself; the node reads at most every two seconds and serves the cached
//! values, and a failed read serves the last good reading for up to 30
//! seconds before read() reports a failure.
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
    #[serde(default = "default_data_pin")]
    data_pin: u32,
    #[serde(default = "default_sck_pin")]
    sck_pin: u32,
    #[serde(default)]
    gpio_chip: u32,
    #[serde(default = "default_sensor_name")]
    sensor_name: String,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
    #[serde(default = "default_ssl_cert")]
    ssl_cert: String,
    #[serde(default = "default_ssl_key")]
    ssl_key: String,
}

fn default_data_pin() -> u32 {
    4
}
fn default_sck_pin() -> u32 {
    5
}
fn default_sensor_name() -> String {
    "SHT11".into()
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

/// The hardware reader: throttles polls to one measurement every two
/// seconds (the self-heating limit), serves the cached values in between,
/// and reports a failure only after MAX_AGE without a successful read.
#[cfg(target_os = "linux")]
mod hw {
    use super::*;
    use driver::{sht1x_humidity, sht1x_temperature, Sht1xBus, CMD_HUMIDITY, CMD_TEMPERATURE};
    use std::time::{Duration, Instant};

    const MIN_INTERVAL: Duration = Duration::from_secs(2);
    const MAX_AGE: Duration = Duration::from_secs(30);

    pub fn reader(chip: u32, data_pin: u32, sck_pin: u32) -> Result<Reader, String> {
        let bus = Sht1xBus::open(&format!("/dev/gpiochip{chip}"), data_pin, sck_pin)?;
        let mut cached: Option<Payload> = None;
        let mut cached_at = Instant::now();
        let mut attempted_at = Instant::now() - MIN_INTERVAL;

        Ok(Box::new(move || {
            let now = Instant::now();
            if now.duration_since(attempted_at) >= MIN_INTERVAL {
                attempted_at = now;
                match bus
                    .measure(CMD_TEMPERATURE)
                    .and_then(|raw_t| bus.measure(CMD_HUMIDITY).map(|raw_h| (raw_t, raw_h)))
                {
                    Ok((raw_temperature, raw_humidity)) => {
                        let temperature = sht1x_temperature(raw_temperature);
                        let mut p = Payload::new();
                        p.insert("temperature".into(), json!(round1(temperature)));
                        p.insert(
                            "humidity".into(),
                            json!(round1(sht1x_humidity(raw_humidity, temperature))),
                        );
                        cached = Some(p);
                        cached_at = now;
                    }
                    Err(err) => {
                        // A single failed transfer is normal on a long
                        // cable; resynchronise and keep serving the cache.
                        println!("SHT1x read failed (serving cache): {err}");
                        let _ = bus.connection_reset();
                    }
                }
            }

            match &cached {
                Some(p) if now.duration_since(cached_at) <= MAX_AGE => Some(p.clone()),
                _ => None,
            }
        }))
    }
}

/// Emulation: a comfortable indoor climate drifting on slow sines around
/// 22 °C / 45 %RH, plus a little measurement noise (like the Python node).
fn emulated_reader() -> Reader {
    Box::new(|| {
        let t = now_secs();
        let mut p = Payload::new();
        p.insert(
            "temperature".into(),
            json!(round1(22.0 + 2.0 * (t / 60.0).sin() + uniform(-0.1, 0.1))),
        );
        p.insert(
            "humidity".into(),
            json!(round1(45.0 + 8.0 * (t / 97.0).sin() + uniform(-0.5, 0.5))),
        );
        Some(p)
    })
}

fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(t), Some(h)) = (full.get("temperature"), full.get("humidity")) {
        p.insert("temp".into(), t.clone());
        p.insert("hum".into(), h.clone());
    }
    p
}

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);
    // The configured chip name is the node identity on every transport.
    let sensor_name: &'static str = Box::leak(cfg.sensor_name.clone().into_boxed_str());

    let reader: Reader = if cfg.emulation {
        println!("Emulation mode: generating {sensor_name} readings without hardware");
        emulated_reader()
    } else {
        println!(
            "Initializing {sensor_name} sensor (DATA=GPIO {}, SCK=GPIO {})...",
            cfg.data_pin, cfg.sck_pin
        );
        #[cfg(target_os = "linux")]
        {
            match hw::reader(cfg.gpio_chip, cfg.data_pin, cfg.sck_pin) {
                Ok(reader) => reader,
                Err(err) => {
                    println!("Error: {err}");
                    exit(1);
                }
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            println!("Error: the SHT1x bit-bang requires Linux (set \"emulation\": true elsewhere)");
            exit(1);
        }
    };
    let reader = Arc::new(Mutex::new(reader));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let host = hostname.clone();
            let build_payload = Box::new(move || {
                let data = (reader.lock().unwrap())()?;
                let mut p = Payload::new();
                p.insert("sensor".into(), json!(sensor_name));
                p.insert("host".into(), json!(host));
                for (k, v) in data {
                    p.insert(k, v);
                }
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                sensor_name,
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
                sensor_name,
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
        sensor_name,
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
