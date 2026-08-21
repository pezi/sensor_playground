//! Sensor Playground Sensor Node — CozIR CO2 Sensor (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a CozIR-A sensor (temperature,
//! humidity, CO2). Unlike the other environment sensors the CozIR is not
//! an I2C device: it talks a simple ASCII command protocol over a
//! 9600-baud UART (serial).
//!
//! Protocol (see the Python node and dart_periphery's serial_cozir.dart):
//!     M 4164\r\n   select humidity, temperature and CO2 output fields
//!     K 2\r\n      polling mode
//!     Q\r\n        request one measurement:
//!                  "H 00495 T 01234 Z 06399" -> 49.5 %RH, 23.4 degC, 639.9 ppm
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

mod serial;

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

/// Parse one measurement line: "H 00495 T 01234 Z 06399" (a leading space
/// may occur). Plain scanning instead of a regex dependency.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_measurement(raw: &str) -> Option<Payload> {
    let mut hum = None;
    let mut temp = None;
    let mut co2 = None;
    let mut tokens = raw.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        let slot = match token {
            "H" => &mut hum,
            "T" => &mut temp,
            "Z" => &mut co2,
            _ => continue,
        };
        if let Some(value) = tokens.peek().and_then(|v| v.parse::<i64>().ok()) {
            *slot = Some(value);
            tokens.next();
        }
    }
    let (hum, temp, co2) = (hum?, temp?, co2?);
    let mut p = Payload::new();
    p.insert("temperature".into(), json!(round1((temp - 1000) as f64 / 10.0)));
    p.insert("humidity".into(), json!(round1(hum as f64 / 10.0)));
    p.insert("co2".into(), json!(round1(co2 as f64 / 10.0)));
    Some(p)
}

type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating CozIR readings without hardware");
        // A quiet indoor room: slow sines for temperature and humidity, a
        // bounded random walk between 400 and 1500 ppm for the CO2.
        let mut co2 = 600.0f64;
        return Box::new(move || {
            let t = now_secs();
            co2 = (co2 + uniform(-15.0, 15.0)).clamp(400.0, 1500.0);
            let mut p = Payload::new();
            p.insert("temperature".into(), json!(round1(22.0 + 2.0 * (t / 60.0).sin() + uniform(-0.1, 0.1))));
            p.insert("humidity".into(), json!(round1(45.0 + 8.0 * (t / 97.0).sin() + uniform(-0.5, 0.5))));
            p.insert("co2".into(), json!(round1(co2)));
            Some(p)
        });
    }
    println!("Initializing CozIR sensor on {}...", cfg.serial_port);
    #[cfg(target_os = "linux")]
    {
        let mut port = match serial::SerialPort::open(&cfg.serial_port) {
            Ok(port) => port,
            Err(err) => {
                println!("Error: opening {}: {err}", cfg.serial_port);
                exit(1);
            }
        };
        // Select the humidity, temperature and CO2 output fields, then
        // switch to polling mode (one measurement per Q command).
        let _ = port.write_all(b"M 4164\r\n");
        let _ = port.write_all(b"K 2\r\n");
        port.flush_input();
        Box::new(move || {
            port.flush_input();
            port.write_all(b"Q\r\n").ok()?;
            parse_measurement(&port.read_line(64))
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the CozIR serial port requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(t), Some(h), Some(c)) = (
        full.get("temperature"),
        full.get("humidity"),
        full.get("co2"),
    ) {
        p.insert("temp".into(), t.clone());
        p.insert("hum".into(), h.clone());
        p.insert("co2".into(), c.clone());
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
                p.insert("sensor".into(), json!("COZIR"));
                p.insert("host".into(), json!(host));
                for (k, v) in data.unwrap_or_default() {
                    p.insert(k, v);
                }
                // Payloads carrying no reading are dropped by the transport
                // contract; publish only when the sensor answered.
                if p.len() > 2 {
                    Some(p)
                } else {
                    None
                }
            });
            if let Err(err) =
                common::ble::run_ble("COZIR", cfg.api_key, common::ble::BleRole::poll(build_payload))
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
                "COZIR",
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
        wifi::run_rest_server("COZIR", cfg.api_key, hostname, read, &cfg.ssl_cert, &cfg.ssl_key)
    {
        println!("Error: {err}");
        exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_measurement_line() {
        let p = parse_measurement(" H 00495 T 01234 Z 06399\r\n").unwrap();
        assert_eq!(p["temperature"], 23.4);
        assert_eq!(p["humidity"], 49.5);
        assert_eq!(p["co2"], 639.9);
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_measurement("").is_none());
        assert!(parse_measurement("H 123 T xx").is_none());
    }
}
