//! Sensor Playground Sensor Node — PAJ7620 Grove Gesture (Rust)
//!
//! Implements the *push* variant of the Sensor Playground Sensor
//! Interface on single-board computers (Raspberry Pi & co.) with a Grove
//! Gesture sensor (PAJ7620U2). The node polls the sensor over I2C and
//! pushes one JSON message ({"gesture": "forward"}) per detected
//! gesture.
//!
//! The gesture strings match the app's Gesture enum names. Like the
//! grove.py driver the Python node uses, the nine basic gestures are
//! reported; the combined gestures (forwardBackward, rightLeft, ...) of
//! the ESP32 sketch are not detected.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE each gesture arrives as a notify on
//!   the data characteristic; the node takes no commands.
//!
//! Set "emulation": true in config.json to generate plausible readings
//! without the sensor hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod driver;

use common::{config, uniform, wifi, ws::WsPushServer, Payload};
use driver::GESTURE_NAMES;
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(100);

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

/// The detected gesture name, or None if nothing happened.
type Reader = Box<dyn FnMut() -> Result<Option<&'static str>, String> + Send>;

fn make_reader(cfg: &Config) -> Reader {
    if cfg.emulation {
        println!("Emulation mode: generating PAJ7620 gestures without hardware");
        // One random gesture from the gesture map every three to five
        // seconds; polls in between return no gesture (like the Python
        // node).
        let mut next_at = Instant::now() + Duration::from_secs_f64(uniform(3.0, 5.0));
        return Box::new(move || {
            if Instant::now() < next_at {
                return Ok(None);
            }
            next_at = Instant::now() + Duration::from_secs_f64(uniform(3.0, 5.0));
            Ok(Some(GESTURE_NAMES[fastrand::usize(..GESTURE_NAMES.len())]))
        });
    }
    println!("Initializing PAJ7620 gesture sensor...");
    #[cfg(target_os = "linux")]
    {
        let mut dev = match driver::hw::Paj7620::new(cfg.i2c_bus) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        Box::new(move || dev.read_gesture())
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real PAJ7620 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

// -- Gesture tick -------------------------------------------------------------

/// One poll step; returns {"gesture": ...} whenever a gesture was
/// detected. Both transports funnel through this.
struct GestureTick {
    read: Reader,
}

impl GestureTick {
    fn new(read: Reader) -> Self {
        Self { read }
    }

    fn tick(&mut self) -> Option<Payload> {
        let name = match (self.read)() {
            Ok(name) => name?,
            Err(err) => {
                println!("Sensor read failed: {err}");
                exit(1);
            }
        };
        println!("Gesture: {name}");
        let mut p = Payload::new();
        p.insert("gesture".into(), json!(name));
        Some(p)
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
            let mut tick = GestureTick::new(read);
            if let Err(err) = common::ble::run_ble(
                "PAJ7620",
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick.tick()),
                    interval: POLL_INTERVAL,
                    on_command: None, // pure push node: the app never writes
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
        |_send: &mut dyn FnMut(&Payload)| {}, // a new client waits for the next gesture
        |_message: &str| {},
    ));

    {
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            if let Err(err) = wifi::run_discovery_listener("PAJ7620", &hostname, wifi::WS_PORT, None)
            {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    // Gesture loop: polls the sensor and pushes each detected gesture.
    {
        let server = server.clone();
        std::thread::spawn(move || {
            let mut tick = GestureTick::new(read);
            loop {
                if let Some(payload) = tick.tick() {
                    server.broadcast(&payload);
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        });
    }

    if let Err(err) = server.listen_and_serve() {
        println!("Error: {err}");
        exit(1);
    }
}
