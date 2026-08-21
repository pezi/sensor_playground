//! Sensor Playground Sensor Node — DHT11 / Grove Temperature & Humidity (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a DHT11 sensor (temperature,
//! humidity) — the blue Grove Temperature & Humidity Sensor module. The
//! DHT22 (Grove "Pro" module, white) speaks the same single-wire protocol
//! with better resolution and has its own node in ../dht22/. Setting
//! "sensor_name": "DHT22" here remains supported for older configurations.
//!
//! The single-wire protocol is timing-critical (26-70 µs pulses), so the
//! pin is read via kernel-timestamped GPIO edge events (gpiocdev) — a
//! userspace polling loop could never tell the pulse widths apart. Reads
//! occasionally fail even on a healthy sensor; the node retries and
//! serves the last good reading for up to 30 seconds, so a single failed
//! read does not surface as an error.
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

mod gpio;

#[cfg(target_os = "linux")]
use common::round1;
use common::{config, now_secs, uniform, wifi, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;
#[cfg(target_os = "linux")]
use std::time::Instant;

// -- Single-wire frame ---------------------------------------------------------

/// Start signal: hold the line low for >=18 ms (DHT11; the DHT22 needs
/// less). thread::sleep's overshoot is harmless — the signal only has a
/// minimum width.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const START_SIGNAL: Duration = Duration::from_millis(18);

/// The response preamble (~240 µs) plus 40 bits (<= ~4.8 ms) are long
/// over before this; a quiet line simply times out.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const READ_TIMEOUT: Duration = Duration::from_millis(25);

/// Falling edges of one frame: the response preamble, 40 bit slots, and
/// the sensor's final low pull before releasing the bus.
const FALLING_EDGES: usize = 42;

/// Between a 0 bit's ~76-78 µs and a 1 bit's ~120 µs falling-to-falling
/// interval (each bit slot is a 50 µs low phase plus a 26-28 µs (0) or
/// 70 µs (1) high phase).
const BIT_THRESHOLD_NS: u64 = 100_000;

/// Turns the falling-edge timestamps of one read into the sensor's five
/// payload bytes, MSB first, and verifies the checksum (sum of the first
/// four bytes, low byte). Only the last 41 edges are used (40
/// falling-to-falling intervals), so a missed or extra edge at the start
/// — the response preamble, or noise from the host releasing the line —
/// does not shift the bits.
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

/// Decodes the five payload bytes into (temperature [°C], humidity
/// [%RH]). The DHT11 sends whole values in bytes 0 and 2; the DHT22
/// sends 16-bit tenths, temperature as sign bit + magnitude — the same
/// decoding adafruit-circuitpython-dht applies for the Python node.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn convert(buf: [u8; 5], dht22: bool) -> (f64, f64) {
    if !dht22 {
        return (f64::from(buf[2]), f64::from(buf[0]));
    }
    let humidity = f64::from((u16::from(buf[0]) << 8) | u16::from(buf[1])) / 10.0;
    let mut temperature = f64::from((u16::from(buf[2] & 0x7F) << 8) | u16::from(buf[3])) / 10.0;
    if buf[2] & 0x80 != 0 {
        temperature = -temperature;
    }
    (temperature, humidity)
}

// -- Sensor -------------------------------------------------------------------

/// Polls are throttled to one hardware read every two seconds — the chip
/// samples at most once per second (DHT11) / once per two seconds
/// (DHT22).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MIN_INTERVAL: Duration = Duration::from_secs(2);

/// A failed read keeps the previous values; the cache goes stale — and
/// read() reports a failure — only after MAX_AGE without a successful
/// read.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const MAX_AGE: Duration = Duration::from_secs(30);

/// Reads a DHT11/DHT22 with the Python node's throttle-and-cache policy.
#[cfg(target_os = "linux")]
struct DhtSensor {
    line: gpio::DhtLine,
    dht22: bool,
    cached: Option<(f64, f64)>, // (temperature, humidity)
    cached_at: Option<Instant>,
    attempted_at: Option<Instant>,
}

#[cfg(target_os = "linux")]
impl DhtSensor {
    /// The full-key readings for the REST API, or None when the sensor
    /// has not answered for a while.
    fn read(&mut self) -> Option<Payload> {
        let now = Instant::now();
        if self
            .attempted_at
            .is_none_or(|t| now.duration_since(t) >= MIN_INTERVAL)
        {
            self.attempted_at = Some(now);
            match self.line.sample().and_then(|f| decode_frame(&f)) {
                Ok(buf) => {
                    self.cached = Some(convert(buf, self.dht22));
                    self.cached_at = Some(now);
                }
                // Single-wire reads fail now and then; the cache covers it.
                Err(err) => println!("DHT read failed (retrying): {err}"),
            }
        }

        let (temperature, humidity) = self.cached?;
        if now.duration_since(self.cached_at?) > MAX_AGE {
            return None;
        }
        let mut p = Payload::new();
        p.insert("temperature".into(), json!(round1(temperature)));
        p.insert("humidity".into(), json!(round1(humidity)));
        Some(p)
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
    "DHT11".into()
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

/// The full-key REST reading, or None on a failed read. Emulation: a
/// comfortable indoor climate drifting on slow sines around 22 °C /
/// 45 %RH, quantised to the DHT11's whole-degree / whole-percent
/// resolution (like the Python node).
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!(
            "Emulation mode: generating {} readings without hardware",
            cfg.sensor_name
        );
        return Box::new(|| {
            let t = now_secs();
            let mut p = Payload::new();
            p.insert(
                "temperature".into(),
                json!((22.0 + 2.0 * (t / 60.0).sin()).round() as i64),
            );
            p.insert(
                "humidity".into(),
                json!((45.0 + 8.0 * (t / 97.0).sin() + uniform(-0.5, 0.5)).round() as i64),
            );
            Some(p)
        });
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
        let mut sensor = DhtSensor {
            line,
            dht22: cfg.sensor_name.eq_ignore_ascii_case("DHT22"),
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
    let mut p = Payload::new();
    if let (Some(t), Some(h)) = (full.get("temperature"), full.get("humidity")) {
        p.insert("temp".into(), t.clone());
        p.insert("hum".into(), h.clone());
    }
    p
}

// -- Main --------------------------------------------------------------------

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
            // Like the Python node, the BLE payload keeps its identity
            // keys even while the sensor is not answering.
            let build_payload = Box::new(move || {
                let mut p = Payload::new();
                p.insert("sensor".into(), json!(sensor_name));
                p.insert("host".into(), json!(host));
                if let Some(data) = (reader.lock().unwrap())() {
                    for (k, v) in data {
                        p.insert(k, v);
                    }
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

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Completes a 4-byte payload with the sensor's checksum (sum of the
    /// four bytes, low byte).
    fn with_checksum(b0: u8, b1: u8, b2: u8, b3: u8) -> [u8; 5] {
        [b0, b1, b2, b3, b0.wrapping_add(b1).wrapping_add(b2).wrapping_add(b3)]
    }

    /// The kernel-timestamped falling edges of one frame: the response
    /// preamble (80 µs low + 80 µs high), then 40 bit slots of 50 µs low
    /// plus ~28 µs (0) or ~70 µs (1) high, closed by the sensor's final
    /// low pull — 42 falling edges, ~78 µs falling-to-falling for a 0
    /// and ~120 µs for a 1.
    fn falling_train(buf: [u8; 5]) -> Vec<u64> {
        let mut ts = 5_000_000u64; // arbitrary start of the response preamble
        let mut train = vec![ts];
        ts += 160_000; // preamble: 80 µs low + 80 µs high
        train.push(ts);
        for i in 0..40 {
            ts += if buf[i / 8] & (1 << (7 - i % 8)) != 0 {
                120_000
            } else {
                78_000
            };
            train.push(ts);
        }
        train
    }

    #[test]
    fn decode_frame_dht11() {
        let want = with_checksum(45, 0, 22, 0); // 45 %RH, 22 °C
        let got = decode_frame(&falling_train(want)).unwrap();
        assert_eq!(got, want);
        assert_eq!(convert(got, false), (22.0, 45.0));
    }

    #[test]
    fn decode_frame_dht22() {
        let want = with_checksum(0x02, 0x8C, 0x01, 0x01); // 652 -> 65.2 %RH, 257 -> 25.7 °C
        let got = decode_frame(&falling_train(want)).unwrap();
        assert_eq!(got, want);
        let (temperature, humidity) = convert(got, true);
        assert!((temperature - 25.7).abs() < 1e-9);
        assert!((humidity - 65.2).abs() < 1e-9);
    }

    /// The DHT22 sends negative temperatures as sign bit + magnitude,
    /// not two's complement.
    #[test]
    fn convert_dht22_negative_temperature() {
        let buf = with_checksum(0x01, 0x90, 0x80, 0x65); // 40.0 %RH, -10.1 °C
        let (temperature, humidity) = convert(buf, true);
        assert!((temperature - (-10.1)).abs() < 1e-9);
        assert!((humidity - 40.0).abs() < 1e-9);
    }

    /// Extra edges before the frame — a partially caught preamble, or
    /// noise from the host releasing the line — must not shift the bits:
    /// the decoder anchors on the last 41 falling edges.
    #[test]
    fn decode_frame_trims_leading_edges() {
        let want = with_checksum(0x02, 0x8C, 0x00, 0xFA); // 65.2 %RH, 25.0 °C
        let mut train = vec![4_800_000, 4_900_000];
        train.extend(falling_train(want));
        assert_eq!(decode_frame(&train).unwrap(), want);
    }

    #[test]
    fn decode_frame_checksum_mismatch() {
        let mut buf = with_checksum(45, 0, 22, 0);
        buf[4] += 1; // corrupt the checksum byte
        assert!(decode_frame(&falling_train(buf))
            .unwrap_err()
            .contains("checksum"));
    }

    #[test]
    fn decode_frame_incomplete() {
        let train = falling_train(with_checksum(45, 0, 22, 0));
        assert!(decode_frame(&train[..40])
            .unwrap_err()
            .contains("incomplete"));
    }
}
