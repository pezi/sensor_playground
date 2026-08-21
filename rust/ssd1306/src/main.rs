//! Sensor Playground Display Node — SSD1306 128x64 OLED (Rust)
//!
//! Implements the *display* variant of the Sensor Playground Sensor
//! Interface on single-board computers (Raspberry Pi & co.) with an
//! SSD1306 I2C OLED. Unlike sensor nodes this node consumes data: the app
//! pushes one JSON command per action over the WebSocket and the node
//! draws it on the panel.
//!
//!     {"id": 1, "image": "<base64>"}   show a bitmap (1024 bytes, below)
//!     {"id": 2, "clear": true}         blank the display
//!
//! The node acknowledges each applied command with the matching id, or
//! returns an error ACK when validation or the display write fails.
//!
//! Bitmap format (matches the dart_periphery SSD1306 example): 128x64
//! pixels, 1 bit per pixel, packed horizontally row by row — 16 bytes per
//! row, MSB of each byte is the leftmost pixel (the
//! https://javl.github.io/image2cpp/ "horizontal" byte orientation). The
//! node transposes this into the SSD1306 native page format before writing
//! it over I2C.
//!
//! - WebSocket server (ws://) on port 9132, X-Api-Key checked on the
//!   handshake + UDP discovery on port 9133 (default), or
//! - BLE GATT server ("transport": "ble" in config.json), Linux only.
//!
//! Over BLE the same two actions arrive as binary writes to the command
//! characteristic instead of as JSON, because one ATT write carries at most
//! MTU-3 bytes and a frame is 1024 of them (see FrameAssembler):
//!
//!     0x01 <offset:u16 big-endian> <bytes...>   stage a chunk
//!     0x02                                      draw the staged frame
//!     0x03                                      blank the display
//!
//! Set "emulation": true to run without a panel.
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod commands;
mod display;

use common::{config, wifi, ws::WsPushServer, Payload};
use display::Display;
use serde::Deserialize;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

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
    "0x3C".into()
}
fn default_transport() -> String {
    "wifi".into()
}

/// Parse the hex address string the config carries ("0x3C"), the same form
/// the Python node parses with int(str, 16).
fn parse_i2c_address(text: &str) -> Result<u16, String> {
    u16::from_str_radix(text.trim_start_matches("0x").trim_start_matches("0X"), 16)
        .map_err(|_| format!("i2c_address {text:?} is not a hex address like \"0x3C\""))
}

fn make_display(cfg: &Config) -> Box<dyn Display> {
    if cfg.emulation {
        println!("Emulation mode: reporting frames without hardware");
        return Box::new(display::EmulatedDisplay);
    }
    let address = match parse_i2c_address(&cfg.i2c_address) {
        Ok(address) => address,
        Err(err) => {
            println!("Error: {err}");
            exit(1);
        }
    };
    println!("Initializing SSD1306 display...");
    #[cfg(target_os = "linux")]
    {
        match display::hw::Ssd1306::new(cfg.i2c_bus, address) {
            Ok(panel) => Box::new(panel),
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = address;
        println!("Error: the real SSD1306 requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
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

    let panel: Arc<Mutex<Box<dyn Display>>> = Arc::new(Mutex::new(make_display(&cfg)));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let command_panel = panel.clone();
            let assembler = Mutex::new(commands::FrameAssembler::new(display::FRAME_SIZE));
            let result = common::ble::run_ble(
                "SSD1306",
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    // A display produces nothing to publish; the loop only
                    // keeps the transport alive for incoming commands.
                    tick: Box::new(|| None),
                    interval: std::time::Duration::from_millis(100),
                    on_command: Some(Box::new(move |packet| {
                        let mut assembler = assembler.lock().unwrap();
                        let action = match assembler.feed(packet) {
                            Ok(action) => action,
                            Err(err) => {
                                println!("BLE command failed: {err}");
                                return;
                            }
                        };
                        let mut panel = command_panel.lock().unwrap();
                        let result = match action {
                            commands::FrameAction::Clear => {
                                println!("Command: clear");
                                panel.clear()
                            }
                            commands::FrameAction::Show => {
                                println!("Command: image");
                                panel.show_bitmap(assembler.frame())
                            }
                            commands::FrameAction::None => Ok(()),
                        };
                        if let Err(err) = result {
                            println!("Display write failed: {err}");
                        }
                    })),
                },
            );
            // Blank the panel rather than leaving a stale image burning.
            let _ = panel.lock().unwrap().clear();
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

    let server = Arc::new(WsPushServer::new_replying(
        cfg.api_key,
        |_send: &mut dyn FnMut(&Payload)| {},
        {
            let panel = panel.clone();
            move |message: &str| {
                Some(commands::handle_json_command(
                    panel.lock().unwrap().as_mut(),
                    message,
                ))
            }
        },
    ));

    {
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            if let Err(err) =
                wifi::run_discovery_listener("SSD1306", &hostname, wifi::WS_PORT, None)
            {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    // Blank the panel rather than leaving a stale image burning.
    {
        let panel = panel.clone();
        std::thread::spawn(move || loop {
            if STOP.load(Ordering::SeqCst) {
                let _ = panel.lock().unwrap().clear();
                println!("Stopped.");
                exit(0);
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        });
    }

    if let Err(err) = server.listen_and_serve() {
        println!("Error: {err}");
        exit(1);
    }
}
