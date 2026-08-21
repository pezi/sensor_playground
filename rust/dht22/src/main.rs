//! Sensor Playground Sensor Node — DHT22 / Grove Temperature & Humidity Pro
//! (Rust)
//!
//! Reads a DHT22/AM2302 from kernel-timestamped GPIO edge events and exposes
//! it through the Sensor Playground Wi-Fi or BLE transport. Hardware reads
//! are limited to the DHT22's two-second sampling interval; transient failures
//! use the last successful reading for up to 30 seconds.

mod gpio;

use common::{config, now_secs, round1, uniform, wifi, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::time::Instant;

// -- Single-wire frame ---------------------------------------------------------

/// DHT22 needs at least 1 ms low; the DHT11-compatible 18 ms signal is also
/// accepted and leaves generous scheduler margin.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const START_SIGNAL: Duration = Duration::from_millis(18);
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const READ_TIMEOUT: Duration = Duration::from_millis(25);
const FALLING_EDGES: usize = 42;
const BIT_THRESHOLD_NS: u64 = 100_000;

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn decode_frame(falling_ns: &[u64]) -> Result<[u8; 5], String> {
    if falling_ns.len() < 41 {
        return Err(format!(
            "incomplete bit train ({} of {FALLING_EDGES} falling edges)",
            falling_ns.len()
        ));
    }
    let bits = &falling_ns[falling_ns.len() - 41..];
    let mut buf = [0u8; 5];
    for i in 0..40 {
        if bits[i + 1].wrapping_sub(bits[i]) >= BIT_THRESHOLD_NS {
            buf[i / 8] |= 1 << (7 - i % 8);
        }
    }
    let sum = buf[..4].iter().map(|&b| u32::from(b)).sum::<u32>() & 0xFF;
    if sum != u32::from(buf[4]) {
        return Err(format!("checksum mismatch ({buf:02x?})"));
    }
    Ok(buf)
}

/// DHT22 humidity and temperature are unsigned 16-bit tenths, except that
/// temperature uses the high bit as a sign flag.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn convert_dht22(buf: [u8; 5]) -> (f64, f64) {
    let humidity = f64::from((u16::from(buf[0]) << 8) | u16::from(buf[1])) / 10.0;
    let mut temperature = f64::from((u16::from(buf[2] & 0x7F) << 8) | u16::from(buf[3])) / 10.0;
    if buf[2] & 0x80 != 0 {
        temperature = -temperature;
    }
    (temperature, humidity)
}

// -- Sensor -------------------------------------------------------------------

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MIN_INTERVAL: Duration = Duration::from_secs(2);
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MAX_AGE: Duration = Duration::from_secs(30);

#[cfg(target_os = "linux")]
struct Dht22Sensor {
    line: gpio::DhtLine,
    cached: Option<(f64, f64)>,
    cached_at: Option<Instant>,
    attempted_at: Option<Instant>,
}

#[cfg(target_os = "linux")]
impl Dht22Sensor {
    fn read(&mut self) -> Option<Payload> {
        let now = Instant::now();
        if self
            .attempted_at
            .is_none_or(|t| now.duration_since(t) >= MIN_INTERVAL)
        {
            self.attempted_at = Some(now);
            match self.line.sample().and_then(|frame| decode_frame(&frame)) {
                Ok(buf) => {
                    self.cached = Some(convert_dht22(buf));
                    self.cached_at = Some(now);
                }
                Err(err) => println!("DHT read failed (retrying): {err}"),
            }
        }

        let (temperature, humidity) = self.cached?;
        if now.duration_since(self.cached_at?) > MAX_AGE {
            return None;
        }
        let mut payload = Payload::new();
        payload.insert("temperature".into(), json!(round1(temperature)));
        payload.insert("humidity".into(), json!(round1(humidity)));
        Some(payload)
    }
}

// -- Config -------------------------------------------------------------------

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_gpio_pin")]
    gpio_pin: u32,
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

fn default_gpio_pin() -> u32 {
    4
}
fn default_sensor_name() -> String {
    "DHT22".into()
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

fn emulated_payload(t: f64, noise: f64) -> Payload {
    let mut payload = Payload::new();
    payload.insert(
        "temperature".into(),
        json!(round1(22.0 + 2.0 * (t / 60.0).sin())),
    );
    payload.insert(
        "humidity".into(),
        json!(round1(45.0 + 8.0 * (t / 97.0).sin() + noise)),
    );
    payload
}

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!(
            "Emulation mode: generating {} readings without hardware",
            cfg.sensor_name
        );
        return Box::new(|| Some(emulated_payload(now_secs(), uniform(-0.5, 0.5))));
    }

    println!(
        "Initializing {} sensor on GPIO {}...",
        cfg.sensor_name, cfg.gpio_pin
    );
    #[cfg(target_os = "linux")]
    {
        let chip = format!("/dev/gpiochip{}", cfg.gpio_chip);
        let line = match gpio::DhtLine::open(&chip, cfg.gpio_pin) {
            Ok(line) => line,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        let mut sensor = Dht22Sensor {
            line,
            cached: None,
            cached_at: None,
            attempted_at: None,
        };
        Box::new(move || sensor.read())
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: GPIO requires Linux (use \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn to_discovery(full: &Payload) -> Payload {
    let mut payload = Payload::new();
    if let (Some(temperature), Some(humidity)) = (full.get("temperature"), full.get("humidity")) {
        payload.insert("temp".into(), temperature.clone());
        payload.insert("hum".into(), humidity.clone());
    }
    payload
}

// -- Main ---------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);
    let sensor_name: &'static str = Box::leak(cfg.sensor_name.clone().into_boxed_str());
    let reader = Arc::new(Mutex::new(make_reader(&cfg)));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let host = hostname.clone();
            let build_payload = Box::new(move || {
                let mut payload = Payload::new();
                payload.insert("sensor".into(), json!(sensor_name));
                payload.insert("host".into(), json!(host));
                if let Some(data) = (reader.lock().unwrap())() {
                    payload.extend(data);
                }
                Some(payload)
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
                Some(full.map(|data| to_discovery(&data)).unwrap_or_default())
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

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn with_checksum(b0: u8, b1: u8, b2: u8, b3: u8) -> [u8; 5] {
        [
            b0,
            b1,
            b2,
            b3,
            b0.wrapping_add(b1).wrapping_add(b2).wrapping_add(b3),
        ]
    }

    fn falling_train(buf: [u8; 5]) -> Vec<u64> {
        let mut timestamp = 5_000_000u64;
        let mut train = vec![timestamp];
        timestamp += 160_000;
        train.push(timestamp);
        for i in 0..40 {
            timestamp += if buf[i / 8] & (1 << (7 - i % 8)) != 0 {
                120_000
            } else {
                78_000
            };
            train.push(timestamp);
        }
        train
    }

    #[test]
    fn decodes_dht22_frame() {
        let want = with_checksum(0x02, 0x8C, 0x01, 0x01);
        let got = decode_frame(&falling_train(want)).unwrap();
        assert_eq!(got, want);
        let (temperature, humidity) = convert_dht22(got);
        assert!((temperature - 25.7).abs() < 1e-9);
        assert!((humidity - 65.2).abs() < 1e-9);
    }

    #[test]
    fn decodes_negative_temperature() {
        let (temperature, humidity) = convert_dht22(with_checksum(0x01, 0x90, 0x80, 0x65));
        assert!((temperature - (-10.1)).abs() < 1e-9);
        assert!((humidity - 40.0).abs() < 1e-9);
    }

    #[test]
    fn trims_leading_edges_and_rejects_bad_frames() {
        let frame = with_checksum(0x02, 0x8C, 0x00, 0xFA);
        let mut train = vec![4_800_000, 4_900_000];
        train.extend(falling_train(frame));
        assert_eq!(decode_frame(&train).unwrap(), frame);

        let mut corrupt = frame;
        corrupt[4] = corrupt[4].wrapping_add(1);
        assert!(decode_frame(&falling_train(corrupt)).is_err());
        assert!(decode_frame(&falling_train(frame)[..40]).is_err());
    }

    #[test]
    fn emulation_uses_one_decimal_precision() {
        let payload = emulated_payload(60.0, 0.0);
        let temperature = payload["temperature"].as_f64().unwrap();
        let humidity = payload["humidity"].as_f64().unwrap();
        assert_eq!(temperature, round1(22.0 + 2.0 * 1.0f64.sin()));
        assert_eq!(humidity, round1(45.0 + 8.0 * (60.0f64 / 97.0).sin()));
    }
}
