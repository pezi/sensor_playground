//! Sensor Playground Sensor Node — Digital Contact Sensor (Rust, push)
//!
//! One generic *push* node for the simple two-state Grove/BakeBit digital
//! sensors that react to an event:
//!
//!     BUTTON     — Grove/BakeBit button      (pressed)
//!     HALL       — Grove Hall sensor         (magnetic field present)
//!     MAGSWITCH  — Grove magnetic switch     (reed switch closed)
//!     PIR        — Grove PIR motion sensor   (motion detected)
//!     VIBRATION  — Grove vibration sensor    (SW-420, vibration)
//!     LINEFINDER — Grove Line Finder         (dark line under the sensor)
//!
//! The node polls the input (debounced) and pushes one JSON message
//! whenever the state changes:
//!
//!     {"active": true}    sensor triggered
//!     {"active": false}   sensor released
//!
//! The input is read from a GPIO character-device line ("interface":
//! "gpio" — /sys/class/gpio is gone in Debian 13). The Arduino-based
//! extension hats the Python node also supports ("interface": "hat") are
//! not implemented in this port.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE each state change arrives as a notify
//!   on the data characteristic; this is a pure push node, so there are
//!   no commands.
//!
//! Set "emulation": true in config.json to generate plausible readings
//! without the sensor hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod gpio;

use common::{config, wifi, ws::WsPushServer, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(20); // 50 Hz input poll
const DEBOUNCE: Duration = Duration::from_millis(30); // stable this long before a change counts

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_sensor_name")]
    sensor_name: String,
    #[serde(default = "default_true")]
    active_low: bool,
    #[serde(default = "default_interface")]
    interface: String,
    #[serde(default = "default_gpio_chip")]
    gpio_chip: String,
    #[serde(default)]
    pin: Option<u32>,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_sensor_name() -> String {
    "BUTTON".into()
}
fn default_true() -> bool {
    true
}
fn default_interface() -> String {
    "gpio".into()
}
fn default_gpio_chip() -> String {
    "/dev/gpiochip0".into()
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Inputs ------------------------------------------------------------------

/// Returns true while the sensor is triggered.
type ReadActiveFn = Box<dyn Fn() -> bool + Send + Sync>;

/// Builds the configured input source.
fn make_input(cfg: &Config) -> ReadActiveFn {
    if cfg.emulation {
        // The contact toggles roughly every five seconds, as if someone
        // slowly pressed and released a button.
        return Box::new(|| (common::now_secs() / 5.0) as u64 % 2 == 0);
    }
    if cfg.interface != "gpio" {
        println!(
            "Error: interface \"{}\" is not supported in the Rust port (only \"gpio\"; use the Python node for Arduino-based hats)",
            cfg.interface
        );
        exit(1);
    }
    #[cfg(target_os = "linux")]
    {
        let Some(pin) = cfg.pin else {
            println!("Error: config.json: pin is required");
            exit(1);
        };
        match gpio::InputLine::open(&cfg.gpio_chip, pin, cfg.active_low) {
            Ok(line) => Box::new(move || line.active()),
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: GPIO requires Linux (use \"emulation\": true elsewhere)");
        exit(1);
    }
}

// -- State tick --------------------------------------------------------------

/// Latest published state, shared with newly connecting clients (None
/// until the first debounced reading).
type SharedState = Arc<Mutex<Option<bool>>>;

fn state_payload(active: bool) -> Payload {
    let mut p = Payload::new();
    p.insert("active".into(), json!(active));
    p
}

/// One debounced input-poll step; returns the state to publish whenever
/// the debounced state changed.
struct StateTick {
    read_active: ReadActiveFn,
    state: SharedState,
    stable: Option<bool>,
    candidate: Option<bool>,
    candidate_since: Instant,
}

impl StateTick {
    fn new(read_active: ReadActiveFn, state: SharedState) -> Self {
        Self {
            read_active,
            state,
            stable: None,
            candidate: None,
            candidate_since: Instant::now(),
        }
    }

    fn tick(&mut self) -> Option<Payload> {
        let active = (self.read_active)();
        if self.candidate != Some(active) {
            self.candidate = Some(active);
            self.candidate_since = Instant::now();
            None
        } else if self.stable != Some(active) && self.candidate_since.elapsed() >= DEBOUNCE {
            self.stable = Some(active);
            *self.state.lock().unwrap() = Some(active);
            println!("active: {active}");
            Some(state_payload(active))
        } else {
            None
        }
    }
}

// -- Shutdown ----------------------------------------------------------------

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn install_signal_handlers() {
    unsafe {
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);
    let sensor_name: &'static str = Box::leak(cfg.sensor_name.clone().into_boxed_str());

    if cfg.emulation {
        println!("Emulation mode: generating {sensor_name} readings without hardware");
    } else {
        println!("Initializing {sensor_name} digital contact sensor...");
    }
    let read_active = make_input(&cfg);
    let state: SharedState = Arc::new(Mutex::new(None));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let mut tick = StateTick::new(read_active, state);
            let result = common::ble::run_ble(
                sensor_name,
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick.tick()),
                    interval: POLL_INTERVAL,
                    on_command: None,
                },
            );
            if let Err(err) = result {
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

    install_signal_handlers();

    // Send the current state to a client right after it connects.
    let server = Arc::new(WsPushServer::new(
        cfg.api_key,
        {
            let state = state.clone();
            move |send: &mut dyn FnMut(&Payload)| {
                if let Some(active) = *state.lock().unwrap() {
                    send(&state_payload(active));
                }
            }
        },
        |_message: &str| {},
    ));

    {
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            if let Err(err) =
                wifi::run_discovery_listener(sensor_name, &hostname, wifi::WS_PORT, None)
            {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    // Input loop: polls the input (debounced) and pushes each change.
    {
        let server = server.clone();
        std::thread::spawn(move || {
            let mut tick = StateTick::new(read_active, state);
            loop {
                if STOP.load(Ordering::SeqCst) {
                    println!("Stopped.");
                    exit(0);
                }
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
