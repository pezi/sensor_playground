//! Sensor Playground Sensor Node — Grove NFC Tag (Rust)
//!
//! Implements the *push* variant of the Sensor Playground Sensor Interface
//! on single-board computers (Raspberry Pi & co.) with a Grove NFC Tag — a
//! passive dual-interface EEPROM (ST M24LR64E-R, 8 KB). A phone or NFC
//! writer stores an NDEF message over the ISO 15693 RF interface; this
//! node reads the same memory over I2C, parses the first NDEF record and
//! pushes one JSON message whenever the content changes:
//!
//!     {"kind": "text", "value": "Hello"}
//!     {"kind": "uri",  "value": "https://seeed.cc"}
//!     {"kind": "data", "value": "DEADBEEF"}      (hex, truncated)
//!     {"kind": "empty"}
//!
//! Unlike the pure event sensors the tag holds state, so the current
//! content is also sent to every client right after it connects (and
//! served on BLE reads).
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE each change arrives as a notify on the
//!   data characteristic; the node takes no commands.
//!
//! Set "emulation": true in config.json to cycle through generated
//! contents without the tag hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod m24lr64;
// Off Linux only the emulation runs, so nothing but the tests parses NDEF.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod ndef;

use common::{config, wifi, ws::WsPushServer, Payload};
use ndef::Content;
use serde::Deserialize;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Seconds between EEPROM polls; an RF write shows up on the next poll.
const POLL_INTERVAL: Duration = Duration::from_secs(1);

/// The emulation advances to the next fake content this often.
const EMULATION_STEP: Duration = Duration::from_secs(15);

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_i2c_bus")]
    i2c_bus: u8,
    #[serde(default = "default_i2c_address")]
    i2c_address: String,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_i2c_bus() -> u8 {
    1
}
fn default_i2c_address() -> String {
    "0x53".into()
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Tag ---------------------------------------------------------------------

/// The polled tag: the hardware EEPROM or the emulation.
type ContentReader = Box<dyn FnMut() -> Result<Content, String> + Send>;

fn make_reader(cfg: &Config) -> ContentReader {
    if cfg.emulation {
        println!("Emulation mode: cycling NFC tag contents without hardware");
        // The same three contents as the Python node, one every 15 s.
        let contents = [
            Content {
                kind: "text",
                value: "Hello from Sensor Playground".into(),
            },
            Content {
                kind: "uri",
                value: "https://wiki.seeedstudio.com/Grove_NFC_Tag/".into(),
            },
            Content {
                kind: "empty",
                value: String::new(),
            },
        ];
        let mut index = 0;
        let mut next_at = Instant::now() + EMULATION_STEP;
        return Box::new(move || {
            if Instant::now() >= next_at {
                index = (index + 1) % contents.len();
                next_at = Instant::now() + EMULATION_STEP;
            }
            Ok(contents[index].clone())
        });
    }
    println!("Opening M24LR64E on i2c bus {}...", cfg.i2c_bus);
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
        let mut tag = match m24lr64::M24lr64Tag::open(cfg.i2c_bus, addr) {
            Ok(tag) => tag,
            Err(err) => {
                println!(
                    "Error: opening the NFC tag on i2c bus {}: {err}",
                    cfg.i2c_bus
                );
                exit(1);
            }
        };
        Box::new(move || tag.read_content().map_err(|err| err.to_string()))
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the real NFC tag requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

// -- Content tick -------------------------------------------------------------

/// One poll step; returns the payload whenever the tag content changed.
/// The last content is kept in `state` so a client that just connected
/// gets it instead of waiting for the next RF write. Both transports
/// funnel through this.
struct ContentTick {
    read: ContentReader,
    state: Arc<Mutex<Option<Content>>>,
}

impl ContentTick {
    fn new(read: ContentReader, state: Arc<Mutex<Option<Content>>>) -> Self {
        Self { read, state }
    }

    fn tick(&mut self) -> Option<Payload> {
        let content = match (self.read)() {
            Ok(content) => content,
            Err(err) => {
                println!("Tag read failed: {err}");
                return None;
            }
        };
        let mut state = self.state.lock().unwrap();
        if state.as_ref() == Some(&content) {
            return None;
        }
        let payload = content.payload();
        println!(
            "Content: {}",
            serde_json::to_string(&serde_json::Value::Object(payload.clone())).unwrap()
        );
        *state = Some(content);
        Some(payload)
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);

    let read = make_reader(&cfg);
    let state: Arc<Mutex<Option<Content>>> = Arc::new(Mutex::new(None));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let mut tick = ContentTick::new(read, state.clone());
            if let Err(err) = common::ble::run_ble(
                "NFCTAG",
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

    // Send the current content to a client right after it connects.
    let server = Arc::new(WsPushServer::new(
        cfg.api_key,
        {
            let state = state.clone();
            move |send: &mut dyn FnMut(&Payload)| {
                if let Some(content) = state.lock().unwrap().as_ref() {
                    send(&content.payload());
                }
            }
        },
        |_message: &str| {}, // push node: incoming messages are ignored
    ));

    {
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            if let Err(err) = wifi::run_discovery_listener("NFCTAG", &hostname, wifi::WS_PORT, None)
            {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    // Content loop: polls the EEPROM and pushes every change.
    {
        let server = server.clone();
        std::thread::spawn(move || {
            let mut tick = ContentTick::new(read, state);
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
