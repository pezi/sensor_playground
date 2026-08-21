//! Sensor Playground Clock Node — Grove 4-Digit Display / TM1637 (Rust)
//!
//! Implements the *actuator* variant of the Sensor Playground Sensor
//! Interface: the app pushes the time to show (and a brightness), and the
//! node reports the state it is actually displaying — which keeps
//! changing on its own, because once a time is set the node advances the
//! minute and blinks the colon autonomously.
//!
//!     app -> node   {"time": "HH:MM"}     set the displayed time (24-hour)
//!                   {"brightness": 0..7}  set the brightness (clamped)
//!     node -> app   {"time": "12:34", "brightness": 3}   current state
//!                   {"time": null, "brightness": 3}      no time set yet
//!
//! The node pushes its state on connect, after every accepted command,
//! and on each minute rollover — never on the colon blink, so state
//! traffic stays at one message a minute. Before the first time set the
//! display shows "--:--".
//!
//! The node owns the state; the app renders what the node last reported
//! rather than what it asked for, so a command that never arrived cannot
//! leave the app showing a time the display does not.
//!
//! The TM1637 bus is bit-banged on two GPIO character-device lines
//! ("interface": "gpio", /dev/gpiochipN — /sys/class/gpio is gone in
//! Debian 13). The Arduino-based extension hats ("interface": "hat") are
//! not supported for this node: one frame needs ~50 line transitions and
//! each hat digital_write is a full I2C transaction, far too slow for a
//! display bus.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE the state arrives as a notify on the
//!   data characteristic and the command as a short binary write on the
//!   command characteristic:
//!
//!     0x01 <hh> <mm>   set the time (rejected unless hh<=23 and mm<=59)
//!     0x02 <0..7>      set the brightness (clamped)
//!     0x03             re-notify the current state
//!
//! Set "emulation": true to run without any hardware at all.
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod driver;
mod gpio;

use common::{config, wifi, ws::WsPushServer, Payload};
use driver::{Display, EmulatedDisplay};
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(50); // 20 Hz clock/blink tick
const COLON_BLINK: Duration = Duration::from_millis(500); // 500 ms on, 500 ms off
const DEFAULT_BRIGHTNESS: i64 = 3;

// -- BLE command framing -----------------------------------------------------

const CMD_SET_TIME: u8 = 0x01; // 0x01 <hh> <mm>
const CMD_BRIGHTNESS: u8 = 0x02; // 0x02 <0..7>
const CMD_STATE_REQUEST: u8 = 0x03; // re-notify the current state

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
    #[serde(default = "default_clk_pin")]
    clk_pin: u32,
    #[serde(default = "default_dio_pin")]
    dio_pin: u32,
    #[serde(default = "default_brightness")]
    brightness: i64,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_sensor_name() -> String {
    "TM1637".into()
}
fn default_interface() -> String {
    "gpio".into()
}
fn default_gpio_chip() -> String {
    "/dev/gpiochip0".into()
}
fn default_clk_pin() -> u32 {
    5
}
fn default_dio_pin() -> u32 {
    6
}
fn default_brightness() -> i64 {
    DEFAULT_BRIGHTNESS
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Display -----------------------------------------------------------------

fn make_display(cfg: &Config) -> Box<dyn Display> {
    if cfg.emulation {
        return Box::new(EmulatedDisplay::new());
    }
    if cfg.interface == "hat" {
        println!(
            "Error: the Arduino-based extension hats cannot drive a TM1637: one \
             frame needs ~50 line transitions and each hat digital_write is a \
             full I2C transaction. Wire the display to Pi GPIOs and use \"gpio\" \
             (a Grove Base Hat's digital ports work — they are wired straight to \
             the Pi)."
        );
        exit(1);
    }
    if cfg.interface != "gpio" {
        println!("Error: unknown interface \"{}\" (use \"gpio\")", cfg.interface);
        exit(1);
    }
    #[cfg(target_os = "linux")]
    {
        match gpio::GpioDisplay::open(&cfg.gpio_chip, cfg.clk_pin, cfg.dio_pin) {
            Ok(display) => Box::new(display),
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

// -- Clock state --------------------------------------------------------------

/// Owns the displayed clock state — the single source of truth this node
/// publishes — and keeps it ticking.
///
/// Every command marks the state as pending publication, even one that
/// does not change it: a client that guessed wrong about the current
/// state would otherwise never be corrected. The colon blink renders but
/// never marks pending; the minute rollover does both.
struct ClockController {
    display: Box<dyn Display>,
    time: Option<(u8, u8)>,
    brightness: u8,
    pending: bool,
    colon_on: bool,
    last_blink: Instant,
    minute_accum: f64,
    last_tick: Option<Instant>,
}

impl ClockController {
    fn new(display: Box<dyn Display>, brightness: i64) -> Self {
        let mut clock = Self {
            display,
            time: None,
            brightness: clamp_brightness(brightness),
            pending: true, // publish the initial state as soon as we serve
            colon_on: false,
            last_blink: Instant::now(),
            minute_accum: 0.0,
            last_tick: None,
        };
        clock.render();
        clock
    }

    fn render(&mut self) {
        self.display.render(self.time, self.colon_on, self.brightness);
    }

    /// Sets the displayed time (24-hour) and restarts the minute phase.
    fn set_time(&mut self, hour: u8, minute: u8) {
        self.time = Some((hour, minute));
        // ":00 seconds" is now, and a lit colon gives immediate feedback.
        self.minute_accum = 0.0;
        self.colon_on = true;
        self.last_blink = Instant::now();
        self.render();
        self.pending = true;
    }

    /// Sets the display brightness, clamped to 0..7.
    fn set_brightness(&mut self, brightness: i64) {
        self.brightness = clamp_brightness(brightness);
        self.render();
        self.pending = true;
    }

    /// Marks the state for re-publication without changing it (the BLE
    /// state request; the WebSocket pushes the state on connect instead).
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn request_state(&mut self) {
        self.pending = true;
    }

    /// Advances the local clock and blinks the colon. Only the minute
    /// rollover marks the state pending; the blink is render-only.
    fn tick(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_tick.unwrap_or(now)).as_secs_f64();
        self.last_tick = Some(now);

        let Some((mut hour, mut minute)) = self.time else {
            return;
        };

        if now.duration_since(self.last_blink) >= COLON_BLINK {
            self.last_blink = now;
            self.colon_on = !self.colon_on;
            self.render();
        }

        self.minute_accum += elapsed;
        if self.minute_accum < 60.0 {
            return;
        }
        self.minute_accum -= 60.0;
        minute += 1;
        if minute >= 60 {
            minute = 0;
            hour += 1;
            if hour >= 24 {
                // midnight rollover
                hour = 0;
            }
        }
        self.time = Some((hour, minute));
        self.render();
        self.pending = true;
    }

    /// Returns true once after each change, clearing the pending flag.
    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// Returns the state payload the transports publish.
    fn state(&self) -> Payload {
        let mut p = Payload::new();
        p.insert(
            "time".into(),
            match self.time {
                Some((hour, minute)) => json!(format!("{hour:02}:{minute:02}")),
                None => Value::Null,
            },
        );
        p.insert("brightness".into(), json!(self.brightness));
        p
    }

    fn close(&mut self) {
        self.display.close();
    }
}

type SharedClock = Arc<Mutex<ClockController>>;

fn clamp_brightness(brightness: i64) -> u8 {
    brightness.clamp(0, 7) as u8
}

// -- Commands ----------------------------------------------------------------

/// Parses an "HH:MM" string, returning (hour, minute).
fn parse_time_text(text: &str) -> Option<(u8, u8)> {
    let bytes = text.as_bytes();
    if bytes.len() != 5 || bytes[2] != b':' {
        return None;
    }
    if !bytes[..2].iter().chain(&bytes[3..]).all(u8::is_ascii_digit) {
        return None;
    }
    let hour = (bytes[0] - b'0') * 10 + (bytes[1] - b'0');
    let minute = (bytes[3] - b'0') * 10 + (bytes[4] - b'0');
    if hour > 23 || minute > 59 {
        return None;
    }
    Some((hour, minute))
}

/// Executes one JSON command pushed by the app over the WebSocket.
fn handle_json_command(clock: &SharedClock, message: &str) {
    let Ok(command) = serde_json::from_str::<Value>(message) else {
        println!("Ignoring malformed command");
        return;
    };

    if let Some(value) = command.get("time") {
        let Some((hour, minute)) = value.as_str().and_then(parse_time_text) else {
            println!("Ignoring invalid time");
            return;
        };
        println!("Command: time {hour:02}:{minute:02}");
        clock.lock().unwrap().set_time(hour, minute);
        return;
    }

    // JSON floats and booleans are not integers, like the Python node's
    // isinstance(int) check.
    let Some(brightness) = command.get("brightness").and_then(Value::as_i64) else {
        println!("Ignoring unknown command");
        return;
    };
    println!("Command: brightness {brightness}");
    clock.lock().unwrap().set_brightness(brightness);
}

/// Executes one binary command packet written over BLE (the transport is
/// Linux-only, but the protocol is portable and unit-tested below).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn handle_ble_command(clock: &SharedClock, packet: &[u8]) {
    match packet {
        [CMD_SET_TIME, hour, minute, ..] if *hour <= 23 && *minute <= 59 => {
            println!("Command: time {hour:02}:{minute:02}");
            clock.lock().unwrap().set_time(*hour, *minute);
        }
        [CMD_SET_TIME, ..] => println!("BLE command failed: invalid time command"),
        [CMD_BRIGHTNESS, value, ..] => {
            println!("Command: brightness {value}");
            clock.lock().unwrap().set_brightness(*value as i64);
        }
        [CMD_BRIGHTNESS] => println!("BLE command failed: brightness without a value byte"),
        [CMD_STATE_REQUEST, ..] => {
            println!("Command: state request");
            clock.lock().unwrap().request_state();
        }
        [opcode, ..] => println!("BLE command failed: unknown opcode {opcode:#04x}"),
        [] => println!("BLE command failed: empty command packet"),
    }
}

// -- State tick --------------------------------------------------------------

/// One clock step; returns the state to publish when a change (from any
/// source) is pending. All sources of change funnel through here — a
/// command handler only mutates the controller, and this is what puts the
/// result on the wire, so the app sees a command echo and a minute
/// rollover the same way. Over BLE the command handlers run inside the
/// transport's D-Bus dispatch, where publishing directly would mean
/// reaching across into the async runtime.
fn tick_state(clock: &SharedClock) -> Option<Payload> {
    let mut clock = clock.lock().unwrap();
    clock.tick();
    if !clock.take_pending() {
        return None;
    }
    let state = clock.state();
    println!(
        "state: {}",
        serde_json::to_string(&Value::Object(state.clone())).unwrap()
    );
    Some(state)
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
        println!("Emulation mode: driving a virtual display without hardware");
    } else {
        println!("Initializing TM1637 clock node...");
    }
    let clock: SharedClock = Arc::new(Mutex::new(ClockController::new(
        make_display(&cfg),
        cfg.brightness,
    )));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let tick_clock = clock.clone();
            let command_clock = clock.clone();
            let result = common::ble::run_ble(
                sensor_name,
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick_state(&tick_clock)),
                    interval: POLL_INTERVAL,
                    on_command: Some(Box::new(move |packet| {
                        handle_ble_command(&command_clock, packet)
                    })),
                },
            );
            // Blank the panel rather than leaving a stale time burning.
            clock.lock().unwrap().close();
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
            let clock = clock.clone();
            move |send: &mut dyn FnMut(&Payload)| {
                send(&clock.lock().unwrap().state());
            }
        },
        {
            let clock = clock.clone();
            move |message: &str| handle_json_command(&clock, message)
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

    // State loop: ticks the clock and publishes every pending state.
    {
        let server = server.clone();
        let clock = clock.clone();
        std::thread::spawn(move || loop {
            if STOP.load(Ordering::SeqCst) {
                // Blank the panel rather than leaving a stale time burning.
                clock.lock().unwrap().close();
                println!("Stopped.");
                exit(0);
            }
            if let Some(payload) = tick_state(&clock) {
                server.broadcast(&payload);
            }
            std::thread::sleep(POLL_INTERVAL);
        });
    }

    if let Err(err) = server.listen_and_serve() {
        println!("Error: {err}");
        exit(1);
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// "HH:MM" only, both fields two digits, 24-hour range.
    #[test]
    fn parse_time_text_accepts_hh_mm() {
        assert_eq!(parse_time_text("00:00"), Some((0, 0)));
        assert_eq!(parse_time_text("12:34"), Some((12, 34)));
        assert_eq!(parse_time_text("23:59"), Some((23, 59)));
        assert_eq!(parse_time_text("24:00"), None);
        assert_eq!(parse_time_text("23:60"), None);
        assert_eq!(parse_time_text("9:05"), None);
        assert_eq!(parse_time_text("09:5"), None);
        assert_eq!(parse_time_text("0905"), None);
        assert_eq!(parse_time_text(""), None);
        assert_eq!(parse_time_text("aa:bb"), None);
        assert_eq!(parse_time_text("12:34:56"), None);
    }

    /// The brightness the chip is given, clamped like the Python node's
    /// max(0, min(7, ...)).
    #[test]
    fn brightness_is_clamped() {
        assert_eq!(clamp_brightness(-5), 0);
        assert_eq!(clamp_brightness(0), 0);
        assert_eq!(clamp_brightness(3), 3);
        assert_eq!(clamp_brightness(7), 7);
        assert_eq!(clamp_brightness(8), 7);
        assert_eq!(clamp_brightness(255), 7);
    }

    /// The state payload the transports publish: "--:--" is a null time,
    /// a set time is zero-padded "HH:MM".
    #[test]
    fn state_payload_matches_the_protocol() {
        let mut clock = ClockController::new(Box::new(EmulatedDisplay::new()), 9);
        assert_eq!(clock.state()["time"], Value::Null);
        assert_eq!(clock.state()["brightness"], json!(7)); // clamped
        assert!(clock.take_pending()); // initial state publishes once
        assert!(!clock.take_pending());

        clock.set_time(9, 5);
        assert!(clock.take_pending());
        assert_eq!(clock.state()["time"], json!("09:05"));

        clock.set_brightness(2);
        assert!(clock.take_pending());
        assert_eq!(clock.state()["brightness"], json!(2));

        // A command that changes nothing still republishes the state.
        clock.request_state();
        assert!(clock.take_pending());
    }

    /// The colon blink renders but never publishes; only a minute
    /// rollover does.
    #[test]
    fn blink_does_not_publish() {
        let mut clock = ClockController::new(Box::new(EmulatedDisplay::new()), 3);
        clock.set_time(12, 34);
        clock.take_pending();
        // Force the blink deadline without waiting half a second.
        clock.last_blink = Instant::now() - COLON_BLINK;
        clock.tick();
        assert!(!clock.colon_on); // set_time lit it, the blink turned it off
        assert!(!clock.take_pending());

        // A full minute of accumulated ticks rolls the clock over.
        clock.minute_accum = 60.0;
        clock.tick();
        assert_eq!(clock.time, Some((12, 35)));
        assert!(clock.take_pending());
    }

    /// The BLE binary command protocol: 0x01 <hh> <mm>, 0x02 <0..7>,
    /// 0x03 (state request). Invalid, short and unknown packets leave the
    /// state untouched.
    #[test]
    fn ble_commands_match_the_protocol() {
        let clock: SharedClock = Arc::new(Mutex::new(ClockController::new(
            Box::new(EmulatedDisplay::new()),
            3,
        )));
        clock.lock().unwrap().take_pending();

        handle_ble_command(&clock, &[CMD_SET_TIME, 12, 34]);
        assert_eq!(clock.lock().unwrap().time, Some((12, 34)));
        assert!(clock.lock().unwrap().take_pending());

        for rejected in [
            vec![CMD_SET_TIME, 24, 0],  // hour out of range
            vec![CMD_SET_TIME, 12, 60], // minute out of range
            vec![CMD_SET_TIME, 12],     // missing the minute byte
            vec![CMD_BRIGHTNESS],       // brightness without a value byte
            vec![0x7F],                 // unknown opcode
            vec![],                     // empty packet
        ] {
            handle_ble_command(&clock, &rejected);
            assert_eq!(clock.lock().unwrap().time, Some((12, 34)));
            assert!(!clock.lock().unwrap().take_pending());
        }

        handle_ble_command(&clock, &[CMD_BRIGHTNESS, 9]); // clamped
        assert_eq!(clock.lock().unwrap().brightness, 7);
        assert!(clock.lock().unwrap().take_pending());

        handle_ble_command(&clock, &[CMD_STATE_REQUEST]);
        assert!(clock.lock().unwrap().take_pending());
    }

    /// Minute, hour and midnight rollovers, all local to the node.
    #[test]
    fn clock_rolls_over_at_midnight() {
        let mut clock = ClockController::new(Box::new(EmulatedDisplay::new()), 3);
        clock.set_time(23, 59);
        clock.minute_accum = 60.0;
        clock.tick();
        assert_eq!(clock.time, Some((0, 0)));
        assert_eq!(clock.state()["time"], json!("00:00"));
    }
}
