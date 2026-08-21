//! Sensor Playground EEPROM Node — AT24C128 (Rust)
//!
//! Implements the *actuator* variant of the Sensor Playground Sensor
//! Interface on single-board computers (Raspberry Pi & co.) with an
//! AT24C128 serial EEPROM (128 Kbit / 16 KB, I2C address 0x50). The app
//! stores a short text on the chip and reads it back at any time; the
//! text survives power cycles of both ends.
//!
//! Like the LED the node is the single source of truth: after a write it
//! reads the chip back and reports the *stored* text, so a failed write
//! cannot leave the app showing a text the chip never held.
//!
//!     app -> node   {"write": "Hello"}   store the text on the chip
//!                   {"read": true}       re-read the chip and push
//!     node -> app   {"text": "Hello"}    stored text (on connect and
//!                                        after every write/read, read
//!                                        from the chip)
//!
//! EEPROM layout (offset 0): magic 'S' 'P', u16 big-endian text length
//! (max 512 bytes), then the UTF-8 text. A chip without the magic (e.g.
//! factory-fresh, all 0xFF) reads as an empty text.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE the stored text arrives as a notify on
//!   the data characteristic carrying the same JSON, and a write is
//!   staged in offset-addressed binary chunks on the command
//!   characteristic (like the SSD1306 bitmap), because a text may not fit
//!   in a single ATT write:
//!
//!     0x01 <offset:u16 big-endian> <bytes...>   stage a chunk of UTF-8 text
//!     0x02 <length:u16 big-endian>              store the staged text
//!     0x03                                      re-read the chip and push
//!
//! Set "emulation": true in config.json to run without the chip (the text
//! then lives in memory only).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

// Only the BLE transport (Linux) assembles chunked commands, so off Linux
// nothing but the tests touches this module.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod ble_command;
// Off Linux only the emulation runs, so nothing but the tests encodes or
// decodes an EEPROM record.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod eeprom;

use common::{config, wifi, ws::WsPushServer, Payload};
use eeprom::{Eeprom, EmulatedEeprom, TEXT_MAX_BYTES};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Seconds between pending-publication polls (commands only mark the
/// state dirty; this loop is what puts it on the wire).
const POLL_INTERVAL: Duration = Duration::from_millis(50);

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_sensor_name")]
    sensor_name: String,
    #[serde(default = "default_i2c_bus")]
    i2c_bus: u8,
    #[serde(default = "default_i2c_address")]
    i2c_address: String,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_sensor_name() -> String {
    "AT24C128".into()
}
fn default_i2c_bus() -> u8 {
    1
}
fn default_i2c_address() -> String {
    "0x50".into()
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Hardware ----------------------------------------------------------------

fn make_eeprom(cfg: &Config) -> Box<dyn Eeprom> {
    if cfg.emulation {
        println!("Emulation mode: storing the text in memory without hardware");
        return Box::new(EmulatedEeprom::new());
    }
    #[cfg(target_os = "linux")]
    {
        let addr = match u16::from_str_radix(cfg.i2c_address.trim_start_matches("0x"), 16) {
            Ok(addr) => addr,
            Err(_) => {
                println!(
                    "Error: config.json: invalid i2c_address {:?}",
                    cfg.i2c_address
                );
                exit(1);
            }
        };
        println!("Opening AT24C128 on i2c bus {}...", cfg.i2c_bus);
        match eeprom::At24c128Eeprom::open(cfg.i2c_bus, addr) {
            Ok(chip) => Box::new(chip),
            Err(err) => {
                println!("Error: opening the EEPROM on i2c bus {}: {err}", cfg.i2c_bus);
                exit(1);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real EEPROM requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

// -- Stored-text state -------------------------------------------------------

/// Owns the stored text — the single source of truth this node publishes.
/// Every command marks the state as pending publication, even one that
/// does not change it: the published text is always a fresh read-back, so
/// the app renders what the chip actually holds.
struct EepromController {
    eeprom: Box<dyn Eeprom>,
    text: String,
    pending: bool,
}

impl EepromController {
    fn new(eeprom: Box<dyn Eeprom>) -> Self {
        let mut controller = Self {
            eeprom,
            text: String::new(),
            pending: false,
        };
        controller.read(); // publish the initial text as soon as we serve
        controller
    }

    /// Stores `text` on the chip and re-reads it.
    fn write(&mut self, text: &[u8]) {
        if text.len() > TEXT_MAX_BYTES {
            println!(
                "Rejecting write of {} bytes (max {TEXT_MAX_BYTES})",
                text.len()
            );
        } else if let Err(err) = self.eeprom.write_text(text) {
            println!("EEPROM write failed: {err}");
        }
        self.read();
    }

    /// Re-reads the chip and marks the text for publication. A failed read
    /// keeps the last known text rather than blanking the app.
    fn read(&mut self) {
        match self.eeprom.read_text() {
            Ok(text) => self.text = text,
            Err(err) => println!("EEPROM read failed: {err}"),
        }
        self.pending = true;
    }

    /// Returns true once after each command, clearing the pending flag.
    fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }
}

type SharedController = Arc<Mutex<EepromController>>;

fn text_payload(text: &str) -> Payload {
    let mut p = Payload::new();
    p.insert("text".into(), json!(text));
    p
}

// -- Commands ----------------------------------------------------------------

/// Executes one JSON command pushed by the app over the WebSocket.
fn handle_json_command(controller: &SharedController, message: &str) {
    let Ok(command) = serde_json::from_str::<serde_json::Value>(message) else {
        println!("Ignoring malformed command");
        return;
    };
    if command.get("read") == Some(&json!(true)) {
        println!("Command: read");
        controller.lock().unwrap().read();
        return;
    }
    let Some(text) = command.get("write").and_then(|v| v.as_str()) else {
        println!("Ignoring command without a string 'write'");
        return;
    };
    println!("Command: write {} characters", text.chars().count());
    controller.lock().unwrap().write(text.as_bytes());
}

/// Feeds one binary command packet written over BLE into the assembler and
/// runs whatever command it completes.
#[cfg(target_os = "linux")]
fn handle_ble_command(
    stream: &Mutex<ble_command::BleCommandStream>,
    controller: &SharedController,
    packet: &[u8],
) {
    let command = match stream.lock().unwrap().feed(packet) {
        Ok(Some(command)) => command,
        Ok(None) => return, // a chunk that only staged text
        Err(err) => {
            println!("BLE command failed: {err}");
            return;
        }
    };
    match command {
        ble_command::BleCommand::Read => {
            println!("Command: read");
            controller.lock().unwrap().read();
        }
        ble_command::BleCommand::Write(text) => {
            println!("Command: write {} bytes", text.len());
            controller.lock().unwrap().write(&text);
        }
    }
}

// -- Text tick ---------------------------------------------------------------

/// One publication step: returns the stored text whenever a command marked
/// it pending. The command handlers run inside the transport's dispatch (a
/// D-Bus callback, over BLE) and only mutate the controller; this is what
/// puts the result on the wire, so both transports behave the same.
fn tick(controller: &SharedController) -> Option<Payload> {
    let mut controller = controller.lock().unwrap();
    if controller.take_pending() {
        println!("text: {:?}", controller.text);
        Some(text_payload(&controller.text))
    } else {
        None
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);
    let sensor_name: &'static str = Box::leak(cfg.sensor_name.clone().into_boxed_str());

    let controller: SharedController = Arc::new(Mutex::new(EepromController::new(make_eeprom(&cfg))));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let tick_controller = controller.clone();
            let command_controller = controller.clone();
            let stream = Mutex::new(ble_command::BleCommandStream::new());
            if let Err(err) = common::ble::run_ble(
                sensor_name,
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick(&tick_controller)),
                    interval: POLL_INTERVAL,
                    on_command: Some(Box::new(move |packet| {
                        handle_ble_command(&stream, &command_controller, packet)
                    })),
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
        {
            // The chip holds state: a client that just connected gets the
            // stored text instead of an empty field.
            let controller = controller.clone();
            move |send: &mut dyn FnMut(&Payload)| {
                send(&text_payload(&controller.lock().unwrap().text));
            }
        },
        {
            let controller = controller.clone();
            move |message: &str| handle_json_command(&controller, message)
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

    // Text loop: publishes the stored text whenever a command marked it
    // pending.
    {
        let server = server.clone();
        let controller = controller.clone();
        std::thread::spawn(move || loop {
            if let Some(payload) = tick(&controller) {
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
