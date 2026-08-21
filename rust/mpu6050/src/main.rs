//! Sensor Playground Sensor Node — MPU6050 (Rust)
//!
//! Implements the Sensor Playground Sensor Interface on single-board
//! computers (Raspberry Pi & co.) with an MPU6050 6-axis IMU. The
//! accelerometer axes are converted into roll / pitch angles plus the
//! total acceleration magnitude (g-force); the on-die temperature sensor
//! is reported as well.
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
/// different periods, the g-force stays around 1 g and the die
/// temperature drifts a few degrees above room temperature (like the
/// Python node).
type Reader = Box<dyn FnMut() -> Option<Payload> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating MPU6050 readings without hardware");
        return Box::new(|| {
            let t = now_secs();
            let mut p = Payload::new();
            p.insert("temperature".into(), json!(round1(36.0 + 2.0 * (t / 60.0).sin())));
            p.insert("roll".into(), json!(round1(8.0 * (t / 7.0).sin() + uniform(-0.3, 0.3))));
            p.insert("pitch".into(), json!(round1(5.0 * (t / 11.0).sin() + uniform(-0.3, 0.3))));
            p.insert("gforce".into(), json!(round2(1.0 + uniform(-0.02, 0.02))));
            Some(p)
        });
    }
    println!("Initializing MPU6050 sensor on /dev/i2c-{}...", cfg.i2c_bus);
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Mpu6050::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let (temperature, roll, pitch, gforce) = dev.read().ok()?;
            let mut p = Payload::new();
            p.insert("temperature".into(), json!(round1(temperature)));
            p.insert("roll".into(), json!(round1(roll)));
            p.insert("pitch".into(), json!(round1(pitch)));
            p.insert("gforce".into(), json!(round2(gforce)));
            Some(p)
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real MPU6050 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

/// The push/notify payload: sensor identity plus the current reading.
fn full_payload(hostname: &str, data: Payload) -> Payload {
    let mut p = Payload::new();
    p.insert("sensor".into(), json!("MPU6050"));
    p.insert("host".into(), json!(hostname));
    for (k, v) in data {
        p.insert(k, v);
    }
    p
}

/// The short-key discovery reading built from the full keys.
fn to_discovery(full: &Payload) -> Payload {
    let mut p = Payload::new();
    if let (Some(t), Some(r), Some(pi), Some(g)) = (
        full.get("temperature"),
        full.get("roll"),
        full.get("pitch"),
        full.get("gforce"),
    ) {
        p.insert("temp".into(), t.clone());
        p.insert("roll".into(), r.clone());
        p.insert("pitch".into(), pi.clone());
        p.insert("gforce".into(), g.clone());
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
                "MPU6050",
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

    // The discovery reply carries short keys; a failed read falls back to
    // the identity-only reply.
    {
        let reader = reader.clone();
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            let read_discovery: wifi::ReadFn = Box::new(move || {
                let full = (reader.lock().unwrap())();
                Some(full.map(|f| to_discovery(&f)).unwrap_or_default())
            });
            if let Err(err) = wifi::run_discovery_listener(
                "MPU6050",
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
