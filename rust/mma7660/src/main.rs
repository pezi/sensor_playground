//! Sensor Playground Sensor Node — MMA7660 (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with a Grove 3-Axis Digital
//! Accelerometer ±1.5g (MMA7660FC). The raw axes are converted into
//! roll / pitch angles plus the total acceleration magnitude (g-force).
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default) — the app streams accelerometers rather than polling
//!   them, or
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

use common::{config, now_secs, round1, round2, uniform, wifi, ws::WsPushServer, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The app streams accelerometers instead of polling them, so readings
/// are pushed at the same 250 ms cadence the BLE transport and the ESP32
/// use (the Python nodes' MOTION_NOTIFY_INTERVAL).
const PUSH_INTERVAL: Duration = Duration::from_millis(250);

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
}

fn default_i2c_bus() -> u8 {
    1
}
fn default_transport() -> String {
    "wifi".into()
}

/// The full-key reading, or None on a failed read. Emulation: a gently
/// rocking, near-level board — roll and pitch follow slow sines with
/// different periods and the g-force stays around 1 g (like the Python
/// node).
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating MMA7660 readings without hardware");
        return Box::new(|| {
            let t = now_secs();
            let mut p = Payload::new();
            p.insert("roll".into(), json!(round1(8.0 * (t / 7.0).sin() + uniform(-0.3, 0.3))));
            p.insert("pitch".into(), json!(round1(5.0 * (t / 11.0).sin() + uniform(-0.3, 0.3))));
            p.insert("gforce".into(), json!(round2(1.0 + uniform(-0.02, 0.02))));
            Some(p)
        });
    }
    println!("Initializing MMA7660 sensor on /dev/i2c-{}...", cfg.i2c_bus);
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Mma7660::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let (roll, pitch, gforce) = dev.read().ok()?;
            let mut p = Payload::new();
            p.insert("roll".into(), json!(round1(roll)));
            p.insert("pitch".into(), json!(round1(pitch)));
            p.insert("gforce".into(), json!(round2(gforce)));
            Some(p)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real MMA7660 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

/// The push/notify payload: sensor identity plus the current reading.
fn full_payload(hostname: &str, data: Payload) -> Payload {
    let mut p = Payload::new();
    p.insert("sensor".into(), json!("MMA7660"));
    p.insert("host".into(), json!(hostname));
    for (k, v) in data {
        p.insert(k, v);
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
                Some(full_payload(&host, data))
            });
            // Motion sensors notify at the 250 ms cadence instead of the
            // 1 Hz default (the Python nodes' MOTION_NOTIFY_INTERVAL).
            if let Err(err) = common::ble::run_ble(
                "MMA7660",
                cfg.api_key,
                common::ble::BleRole::Poll {
                    build_payload,
                    interval: PUSH_INTERVAL,
                },
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

    // The discovery reply carries the same full keys as the WebSocket
    // payload (the Python node reuses read() for discovery); a failed
    // read falls back to the identity-only reply.
    {
        let reader = reader.clone();
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            let read_discovery: wifi::ReadFn =
                Box::new(move || Some((reader.lock().unwrap())().unwrap_or_default()));
            if let Err(err) = wifi::run_discovery_listener(
                "MMA7660",
                &hostname,
                wifi::WS_PORT,
                Some(read_discovery),
            ) {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    let server = Arc::new(WsPushServer::new(cfg.api_key, |_send| {}, |_message| {}));

    // Push a reading every PUSH_INTERVAL, like the ESP32 sketch.
    {
        let server = server.clone();
        std::thread::spawn(move || loop {
            if let Some(data) = (reader.lock().unwrap())() {
                server.broadcast(&full_payload(&hostname, data));
            }
            std::thread::sleep(PUSH_INTERVAL);
        });
    }

    if let Err(err) = server.listen_and_serve() {
        println!("Error: {err}");
        exit(1);
    }
}
