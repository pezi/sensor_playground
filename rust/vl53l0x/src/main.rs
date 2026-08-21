//! Sensor Playground Sensor Node — VL53L0X (Rust)
//!
//! Implements the *push* variant of the Sensor Playground Sensor
//! Interface on single-board computers (Raspberry Pi & co.) with a
//! VL53L0X time-of-flight distance sensor. The node measures continuously
//! and pushes one JSON message ({"distance": <mm>}, null when out of
//! range) whenever the distance changes, or at least once per second as a
//! heartbeat.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE each reading arrives as a notify on
//!   the data characteristic; the node takes no commands.
//!
//! Set "emulation": true in config.json to generate plausible readings
//! without the sensor hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod driver;

use common::{config, now_secs, wifi, ws::WsPushServer, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::Arc;
use std::time::{Duration, Instant};

// -- Publish policy -----------------------------------------------------------

const MEASURE_INTERVAL: Duration = Duration::from_millis(100);
const HEARTBEAT: Duration = Duration::from_secs(1);
const MIN_DELTA_MM: i32 = 3;

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

// -- Sensor ------------------------------------------------------------------

/// The distance in mm, or None when no target is in range. Emulation: a
/// target sweeping back and forth between 100 and 1200 mm (20 s period),
/// occasionally leaving the measuring range (like the Python node).
type Reader = Box<dyn FnMut() -> Result<Option<u16>, String> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating VL53L0X readings without hardware");
        return Box::new(|| {
            if fastrand::f64() < 0.02 {
                return Ok(None);
            }
            let phase = (now_secs() % 20.0) / 20.0;
            let mm = (100.0 + 1100.0 * (1.0 - (2.0 * phase - 1.0).abs())).round() as u16;
            Ok(Some(mm))
        });
    }
    println!("Initializing VL53L0X sensor...");
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Vl53l0x::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || {
            let mm = dev.read_range()?;
            // The VL53L0X reports ~8190 mm when no target is in range.
            Ok((mm > 0 && mm < 8000).then_some(mm))
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real VL53L0X requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

fn distance_payload(mm: Option<u16>) -> Payload {
    let mut p = Payload::new();
    p.insert("distance".into(), json!(mm));
    p
}

// -- Measure tick -------------------------------------------------------------

/// One measurement step of the continuous loop; returns the payload to
/// publish on a change, a range flip, or the heartbeat (like the Python
/// node's measure_loop). Both transports funnel through this.
struct MeasureTick {
    read: Reader,
    last_sent: Option<u16>,
    last_publish: Option<Instant>,
    ever_published: bool,
}

impl MeasureTick {
    fn new(read: Reader) -> Self {
        Self {
            read,
            last_sent: None,
            last_publish: None,
            ever_published: false,
        }
    }

    fn tick(&mut self) -> Option<Payload> {
        let mm = match (self.read)() {
            Ok(mm) => mm,
            Err(err) => {
                println!("I2C read failed: {err}");
                return None;
            }
        };
        let changed = !self.ever_published
            || mm.is_none() != self.last_sent.is_none()
            || matches!((mm, self.last_sent), (Some(a), Some(b))
                if (i32::from(a) - i32::from(b)).abs() >= MIN_DELTA_MM);
        let heartbeat_due = self.last_publish.is_none_or(|t| t.elapsed() >= HEARTBEAT);
        if changed || heartbeat_due {
            self.ever_published = true;
            self.last_sent = mm;
            self.last_publish = Some(Instant::now());
            Some(distance_payload(mm))
        } else {
            None
        }
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);

    let read = make_reader(&cfg);

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let mut tick = MeasureTick::new(read);
            if let Err(err) = common::ble::run_ble(
                "VL53L0X",
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick.tick()),
                    interval: MEASURE_INTERVAL,
                    on_command: None, // pure push node
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

    let server = Arc::new(WsPushServer::new(
        cfg.api_key,
        |_send: &mut dyn FnMut(&Payload)| {}, // a new client waits for the next push
        |_message: &str| {},
    ));

    {
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            if let Err(err) =
                wifi::run_discovery_listener("VL53L0X", &hostname, wifi::WS_PORT, None)
            {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    // Measure loop: publishes on change, range flip, or heartbeat.
    {
        let server = server.clone();
        std::thread::spawn(move || {
            let mut tick = MeasureTick::new(read);
            loop {
                if let Some(payload) = tick.tick() {
                    server.broadcast(&payload);
                }
                std::thread::sleep(MEASURE_INTERVAL);
            }
        });
    }

    if let Err(err) = server.listen_and_serve() {
        println!("Error: {err}");
        exit(1);
    }
}
