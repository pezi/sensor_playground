//! Sensor Playground Sensor Node — Air530 GPS (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a Grove GPS (Air530) module. Like
//! the CozIR the Air530 is not an I2C device: it continuously streams
//! NMEA-0183 sentences over a 9600-baud UART (serial). The node reads one
//! burst per request and parses the fix (port of the dart_periphery
//! NmeaParser, see serial_air530.dart):
//!
//! - GGA sentences (preferred): latitude, longitude, MSL altitude,
//!   satellites in use
//! - GLL sentences (fallback): latitude, longitude only
//! - Sentences with bad checksums are skipped
//!
//! - HTTPS REST API on port 9132 + UDP discovery on port 9133 (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only)
//!
//! Until the module has a position fix the read yields an empty payload —
//! not None — so the REST API answers 200 with a metadata-only body
//! instead of the shared transport's 503 (the Python node's allow_empty
//! behavior; the app then shows its "waiting for satellite fix" screen).
//!
//! Set "emulation": true in config.json to generate plausible readings
//! without the sensor hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

// The parser is exercised by the Linux serial path and by the tests; on
// other platforms only the emulation path runs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod nmea;
mod serial;

use common::{config, now_secs, round1, uniform, wifi, Payload};
use nmea::round6;
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
    #[serde(default = "default_serial_port")]
    serial_port: String,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
    #[serde(default = "default_ssl_cert")]
    ssl_cert: String,
    #[serde(default = "default_ssl_key")]
    ssl_key: String,
}

fn default_serial_port() -> String {
    "/dev/serial0".into()
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
        println!("Emulation mode: generating Air530 readings without hardware");
        // A receiver at St. Stephen's Cathedral in Vienna (48.2085 N,
        // 16.3730 E): the position performs a tiny random walk around the
        // base coordinate, the altitude drifts slowly around 171 m MSL
        // and the satellite count varies between 4 and 12.
        let mut lat = 48.2085f64;
        let mut lon = 16.3730f64;
        return Box::new(move || {
            lat += uniform(-0.0001, 0.0001);
            lon += uniform(-0.0001, 0.0001);
            let mut p = Payload::new();
            p.insert("latitude".into(), json!(round6(lat)));
            p.insert("longitude".into(), json!(round6(lon)));
            p.insert("altitude".into(), json!(round1(171.0 + 5.0 * (now_secs() / 120.0).sin())));
            p.insert("satellites".into(), json!(fastrand::u32(4..=12)));
            Some(p)
        });
    }
    println!("Initializing Air530 GPS on {}...", cfg.serial_port);
    #[cfg(target_os = "linux")]
    {
        let mut port = match serial::SerialPort::open(&cfg.serial_port) {
            Ok(port) => port,
            Err(err) => {
                println!("Error: opening {}: {err}", cfg.serial_port);
                exit(1);
            }
        };
        Box::new(move || {
            port.flush_input();
            // One ~1 Hz NMEA burst; partial first lines fail the checksum
            // and are skipped by the parser. A fixless warm-up burst
            // yields an empty payload (200 metadata-only), never None
            // (503) — a GPS still warming up is not an error.
            Some(nmea::parse_nmea(&port.read_block(512)).unwrap_or_default())
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the Air530 serial port requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(lat), Some(lon)) = (full.get("latitude"), full.get("longitude")) {
        p.insert("lat".into(), lat.clone());
        p.insert("lon".into(), lon.clone());
        if let Some(alt) = full.get("altitude") {
            p.insert("alt".into(), alt.clone());
        }
        if let Some(sats) = full.get("satellites") {
            p.insert("sats".into(), sats.clone());
        }
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
                let data = (reader.lock().unwrap())();
                let mut p = Payload::new();
                p.insert("sensor".into(), json!("AIR530"));
                p.insert("host".into(), json!(host));
                for (k, v) in data.unwrap_or_default() {
                    p.insert(k, v);
                }
                // Like the Python node, publish even without a fix: the
                // metadata-only payload is the "waiting for satellite
                // fix" state, not a failed read.
                Some(p)
            });
            if let Err(err) =
                common::ble::run_ble("AIR530", cfg.api_key, common::ble::BleRole::poll(build_payload))
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
                "AIR530",
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
        wifi::run_rest_server("AIR530", cfg.api_key, hostname, read, &cfg.ssl_cert, &cfg.ssl_key)
    {
        println!("Error: {err}");
        exit(1);
    }
}
