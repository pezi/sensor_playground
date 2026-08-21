//! Sensor Playground Sensor Node — Grove Capacitive Moisture Sensor (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers with a Grove Capacitive Moisture Sensor (Corrosion-Resistant)
//! — an analog probe whose output voltage falls as the soil gets wetter.
//! It reports the soil moisture as a percentage (JSON key `moisture`)
//! mapped linearly between the two calibration points in config.json (raw
//! ADC when dry vs. when wet), alongside the raw reading (`adc`/`adcMax`)
//! for calibrating them.
//!
//! The Raspberry Pi has no analog input, so the probe is read through
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

use common::{config, now_secs, wifi, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};

#[allow(dead_code)]
const HAT_I2C_ADDRESS: u16 = 0x04; // Grove Base Hat (STM32F030 ADC)
#[allow(dead_code)]
const HAT_ADC_BASE: u8 = 0x10; // raw 12-bit value registers, one per channel
const HAT_ADC_MAX: i64 = 4095; // full scale of the hat's 12-bit ADC

// Averaging window; a single ADC read of the probe is noisy.
#[allow(dead_code)]
const SAMPLE_COUNT: u32 = 4;

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
    #[serde(default = "default_adc_dry")]
    adc_dry: i64,
    #[serde(default = "default_adc_wet")]
    adc_wet: i64,
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
fn default_adc_dry() -> i64 {
    2600
}
fn default_adc_wet() -> i64 {
    1100
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

/// Maps a raw ADC reading onto 0-100 % between the dry and wet calibration
/// points (the probe's output falls as the soil gets wetter).
fn percent(raw: i64, adc_dry: i64, adc_wet: i64) -> i64 {
    let span = (adc_dry - adc_wet) as f64;
    let p = 100.0 * (adc_dry - raw) as f64 / span;
    p.clamp(0.0, 100.0).round() as i64
}

/// Returns the averaged raw ADC reader plus the calibration points it maps
/// against (the emulation ignores the config and uses the 12-bit defaults,
/// like the Python node).
type RawReader = Box<dyn FnMut() -> Option<i64> + Send>;

fn make_raw_reader(cfg: &Config) -> (RawReader, i64, i64) {
    if cfg.emulation {
        println!("Emulation mode: generating MOISTURE readings without hardware");
        // A watering cycle: the moisture slowly dries from ~85 % down to
        // ~25 % and jumps back up, on a few-minute loop for easy demoing.
        let (adc_dry, adc_wet) = (2600_i64, 1100_i64);
        let reader: RawReader = Box::new(move || {
            let t = now_secs();
            let cycle = (t % 300.0) / 300.0; // 0 -> 1 over five minutes
            let pct = 85.0 - 60.0 * cycle + 2.0 * (t / 3.0).sin();
            let raw = adc_dry as f64 - (adc_dry - adc_wet) as f64 * pct / 100.0;
            Some(raw.round() as i64)
        });
        return (reader, adc_dry, adc_wet);
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
    if cfg.adc_dry == cfg.adc_wet {
        println!("Error: adc_dry and adc_wet must differ (calibrate!)");
        exit(1);
    }
    println!(
        "Initializing moisture probe on grove hat, channel {} (dry={}, wet={})...",
        cfg.pin, cfg.adc_dry, cfg.adc_wet
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
        let reader: RawReader = Box::new(move || {
            // Averages the raw 12-bit ADC value [0-4095] of the channel.
            let mut total: i64 = 0;
            for _ in 0..SAMPLE_COUNT {
                total += dev.read_word(reg).ok()? as i64;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Some((total as f64 / SAMPLE_COUNT as f64).round() as i64)
        });
        (reader, cfg.adc_dry, cfg.adc_wet)
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

    let (raw_reader, adc_dry, adc_wet) = make_raw_reader(&cfg);
    let raw_reader = Arc::new(Mutex::new(raw_reader));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let host = hostname.clone();
            let raw_reader = raw_reader.clone();
            let build_payload = Box::new(move || {
                let raw = (raw_reader.lock().unwrap())()?;
                let mut p = Payload::new();
                p.insert("sensor".into(), json!("MOISTURE"));
                p.insert("host".into(), json!(host));
                p.insert("moisture".into(), json!(percent(raw, adc_dry, adc_wet)));
                p.insert("adc".into(), json!(raw));
                p.insert("adcMax".into(), json!(HAT_ADC_MAX));
                Some(p)
            });
            if let Err(err) = common::ble::run_ble(
                "MOISTURE",
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
        let raw_reader = raw_reader.clone();
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            // Short-key reading for the discovery reply; identity-only on failure.
            let read_discovery: wifi::ReadFn = Box::new(move || {
                let mut p = Payload::new();
                if let Some(raw) = (raw_reader.lock().unwrap())() {
                    p.insert("moist".into(), json!(percent(raw, adc_dry, adc_wet)));
                }
                Some(p)
            });
            if let Err(err) = wifi::run_discovery_listener(
                "MOISTURE",
                &hostname,
                wifi::HTTPS_PORT,
                Some(read_discovery),
            ) {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    let read: wifi::ReadFn = Box::new(move || {
        let raw = (raw_reader.lock().unwrap())()?;
        let mut p = Payload::new();
        // The raw reading and its full scale help calibrate adc_dry/adc_wet.
        p.insert("moisture".into(), json!(percent(raw, adc_dry, adc_wet)));
        p.insert("adc".into(), json!(raw));
        p.insert("adcMax".into(), json!(HAT_ADC_MAX));
        Some(p)
    });
    if let Err(err) = wifi::run_rest_server(
        "MOISTURE",
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

#[cfg(test)]
mod tests {
    use super::percent;

    // The linear calibration mapping: adc_dry -> 0 %, adc_wet -> 100 %,
    // clamped outside the calibrated span.
    #[test]
    fn percent_maps_between_calibration_points() {
        assert_eq!(percent(2600, 2600, 1100), 0); // dry calibration point
        assert_eq!(percent(1100, 2600, 1100), 100); // wet calibration point
        assert_eq!(percent(1850, 2600, 1100), 50); // midpoint
        assert_eq!(percent(1820, 2600, 1100), 52); // rounds to nearest percent
        assert_eq!(percent(275, 650, 275), 100); // 10-bit calibration points
    }

    #[test]
    fn percent_clamps_outside_span() {
        assert_eq!(percent(3000, 2600, 1100), 0); // drier than dry
        assert_eq!(percent(500, 2600, 1100), 100); // wetter than wet
    }
}
