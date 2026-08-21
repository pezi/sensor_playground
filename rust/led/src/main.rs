//! Sensor Playground LED Node — LED + optional push button (Rust)
//!
//! Implements the *actuator* variant of the Sensor Playground Sensor
//! Interface: the app sends a switch command and the node reports the
//! resulting state back, because the LED can also be toggled by a push
//! button wired to the node itself.
//!
//!     app -> node   {"led": true}     switch on
//!                   {"led": false}    switch off
//!                   {"toggle": true}  flip
//!     node -> app   {"led": true|false}   current state (on connect and
//!                                         after every change)
//!
//! The node owns the state; the app renders what the node last reported.
//!
//! The LED and button are driven from GPIO character-device lines
//! ("interface": "gpio" — /sys/class/gpio is gone in Debian 13). The
//! Arduino-based extension hats the Python node also supports
//! ("interface": "hat") are not implemented in this port.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE the state arrives as a notify on the
//!   data characteristic and the command as a short binary write on the
//!   command characteristic:
//!
//!     0x01 0x00   switch off
//!     0x01 0x01   switch on
//!     0x02        flip
//!
//! Set "button_pin": null for an LED-only node, and "emulation": true to
//! run without any hardware at all.
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

const POLL_INTERVAL: Duration = Duration::from_millis(20); // 50 Hz button poll
const DEBOUNCE: Duration = Duration::from_millis(30);

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_sensor_name")]
    sensor_name: String,
    #[serde(default = "default_interface")]
    interface: String,
    #[serde(default = "default_gpio_chip")]
    gpio_chip: String,
    #[serde(default)]
    led_pin: Option<u32>,
    #[serde(default)]
    led_active_low: bool,
    #[serde(default)]
    button_pin: Option<u32>,
    #[serde(default = "default_true")]
    button_active_low: bool,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_sensor_name() -> String {
    "LED".into()
}
fn default_interface() -> String {
    "gpio".into()
}
fn default_gpio_chip() -> String {
    "/dev/gpiochip0".into()
}
fn default_true() -> bool {
    true
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Hardware ----------------------------------------------------------------

type WriteFn = Box<dyn Fn(bool) + Send + Sync>;
type PressedFn = Box<dyn Fn() -> bool + Send + Sync>;

fn make_hardware(cfg: &Config) -> (WriteFn, PressedFn) {
    if cfg.emulation {
        let write: WriteFn = Box::new(|on| {
            println!("[emulation] LED {}", if on { "on" } else { "off" });
        });
        return (write, Box::new(|| false));
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
        let Some(led_pin) = cfg.led_pin else {
            println!("Error: config.json: led_pin is required");
            exit(1);
        };
        let output = match gpio::OutputLine::open(&cfg.gpio_chip, led_pin, cfg.led_active_low) {
            Ok(line) => line,
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        };
        let write: WriteFn = Box::new(move |on| output.set(on));

        let pressed: PressedFn = match cfg.button_pin {
            Some(pin) => {
                match gpio::InputLine::open(&cfg.gpio_chip, pin, cfg.button_active_low) {
                    Ok(line) => Box::new(move || line.pressed()),
                    Err(err) => {
                        println!("Error: {err}");
                        exit(1);
                    }
                }
            }
            None => Box::new(|| false),
        };
        (write, pressed)
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: GPIO requires Linux (use \"emulation\": true elsewhere)");
        exit(1);
    }
}

// -- LED state ---------------------------------------------------------------

/// Owns the LED state — the single source of truth this node publishes.
/// Every switch marks the state as pending publication, even one that does
/// not change it: a client that guessed wrong about the current state
/// would otherwise never be corrected.
struct LedController {
    write: WriteFn,
    on: bool,
    pending: bool,
}

impl LedController {
    fn new(write: WriteFn) -> Self {
        write(false);
        Self {
            write,
            on: false,
            pending: true, // publish the initial state as soon as we serve
        }
    }

    fn set(&mut self, on: bool) {
        self.on = on;
        (self.write)(on);
        self.pending = true;
    }

    fn toggle(&mut self) {
        let on = !self.on;
        self.set(on);
    }

    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

type SharedLed = Arc<Mutex<LedController>>;

fn state_payload(on: bool) -> Payload {
    let mut p = Payload::new();
    p.insert("led".into(), json!(on));
    p
}

// -- Commands ----------------------------------------------------------------

/// Executes one JSON command pushed by the app over the WebSocket.
fn handle_json_command(led: &SharedLed, message: &str) {
    let Ok(command) = serde_json::from_str::<serde_json::Value>(message) else {
        println!("Ignoring malformed command");
        return;
    };
    if command.get("toggle") == Some(&json!(true)) {
        println!("Command: toggle");
        led.lock().unwrap().toggle();
        return;
    }
    let Some(on) = command.get("led").and_then(|v| v.as_bool()) else {
        println!("Ignoring command without a boolean 'led'");
        return;
    };
    println!("Command: led {}", if on { "on" } else { "off" });
    led.lock().unwrap().set(on);
}

/// Executes one binary command packet written over BLE.
#[cfg(target_os = "linux")]
fn handle_ble_command(led: &SharedLed, packet: &[u8]) {
    match packet {
        [0x02, ..] => {
            println!("Command: toggle");
            led.lock().unwrap().toggle();
        }
        [0x01, state, ..] => {
            let on = *state != 0;
            println!("Command: led {}", if on { "on" } else { "off" });
            led.lock().unwrap().set(on);
        }
        _ => println!("BLE command failed: unknown or short packet"),
    }
}

// -- State tick --------------------------------------------------------------

/// One debounced button-poll step; returns the state to publish when a
/// change (from any source) is pending. Both the app command handlers and
/// the button funnel through this, so the app sees an app-initiated switch
/// and a button press the same way.
struct StateTick {
    led: SharedLed,
    pressed_fn: PressedFn,
    pressed: Option<bool>,
    candidate: Option<bool>,
    candidate_since: Instant,
}

impl StateTick {
    fn new(led: SharedLed, pressed_fn: PressedFn) -> Self {
        Self {
            led,
            pressed_fn,
            pressed: None,
            candidate: None,
            candidate_since: Instant::now(),
        }
    }

    fn tick(&mut self) -> Option<Payload> {
        let is_pressed = (self.pressed_fn)();
        if self.candidate != Some(is_pressed) {
            self.candidate = Some(is_pressed);
            self.candidate_since = Instant::now();
        } else if self.pressed != Some(is_pressed)
            && self.candidate_since.elapsed() >= DEBOUNCE
        {
            let first_reading = self.pressed.is_none();
            self.pressed = Some(is_pressed);
            // The first stable reading only establishes the idle level;
            // toggle on press, not on release, so one press is one toggle.
            if is_pressed && !first_reading {
                println!("Button pressed: toggling LED");
                self.led.lock().unwrap().toggle();
            }
        }

        let mut led = self.led.lock().unwrap();
        if led.take_pending() {
            println!("led: {}", led.on);
            Some(state_payload(led.on))
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
        println!("Emulation mode: switching a virtual LED without hardware");
    } else {
        println!("Initializing LED node...");
    }
    let (write, pressed) = make_hardware(&cfg);
    let led: SharedLed = Arc::new(Mutex::new(LedController::new(write)));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let mut tick = StateTick::new(led.clone(), pressed);
            let command_led = led.clone();
            let result = common::ble::run_ble(
                sensor_name,
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick.tick()),
                    interval: POLL_INTERVAL,
                    on_command: Some(Box::new(move |packet| handle_ble_command(&command_led, packet))),
                },
            );
            // Leave the LED dark rather than stuck on after the node exits.
            led.lock().unwrap().set(false);
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

    let server = Arc::new(WsPushServer::new(
        cfg.api_key,
        {
            let led = led.clone();
            move |send: &mut dyn FnMut(&Payload)| {
                send(&state_payload(led.lock().unwrap().on));
            }
        },
        {
            let led = led.clone();
            move |message: &str| handle_json_command(&led, message)
        },
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

    // State loop: polls the button (debounced) and publishes every change.
    {
        let server = server.clone();
        let led = led.clone();
        std::thread::spawn(move || {
            let mut tick = StateTick::new(led.clone(), pressed);
            loop {
                if STOP.load(Ordering::SeqCst) {
                    // Leave the LED dark rather than stuck on after exit.
                    led.lock().unwrap().set(false);
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
