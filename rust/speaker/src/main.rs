//! Sensor Playground Speaker Node — Grove Speaker (Rust)
//!
//! Implements the *actuator* variant of the Sensor Playground Sensor
//! Interface on single-board computers (Raspberry Pi & co.) with a Grove
//! Speaker — a small amplified loudspeaker on a digital pin, driven with a
//! square wave of the desired pitch. Like the LED the node talks in both
//! directions: the app asks for a tone, the built-in melody, or silence,
//! and the node reports what is *actually* sounding — a tone ends on its
//! own when its duration runs out, so the app follows the node's reports
//! rather than its own taps.
//!
//!     app -> node   {"tone": {"freq": 440, "ms": 400}}   play one tone
//!                   {"melody": true}                     play the melody
//!                   {"stop": true}                       silence
//!     node -> app   {"freq": 440} / {"freq": 0}          what is sounding
//!
//! Over BLE the state arrives as a notify on the data characteristic and
//! the command as a short binary write on the command characteristic:
//!
//!     0x01 <freq:u16 big-endian> <ms:u16 big-endian>   play one tone
//!     0x02                                             play the melody
//!     0x03                                             silence
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only)
//!
//! Set "emulation": true to run without any hardware at all.
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod controller;
mod gpio;
mod output;

use common::{config, wifi, ws::WsPushServer, Payload};
use controller::{handle_json_command, MonotonicClock, SpeakerController};
use serde::Deserialize;
use std::process::exit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How often the playback loop advances tones and publishes changes.
const POLL_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_sensor_name")]
    sensor_name: String,
    #[serde(default = "default_speaker_pin")]
    speaker_pin: u32,
    #[serde(default)]
    gpio_chip: u32,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_sensor_name() -> String {
    "SPEAKER".into()
}
fn default_speaker_pin() -> u32 {
    5
}
fn default_transport() -> String {
    "wifi".into()
}

type SharedSpeaker = Arc<Mutex<SpeakerController>>;

fn make_output(cfg: &Config) -> Box<dyn output::Output> {
    if cfg.emulation {
        println!("Emulation mode: printing tones without hardware");
        return Box::new(output::EmulatedOutput);
    }
    println!("Initializing speaker on GPIO {}...", cfg.speaker_pin);
    #[cfg(target_os = "linux")]
    {
        match output::PwmOutput::open(&format!("/dev/gpiochip{}", cfg.gpio_chip), cfg.speaker_pin) {
            Ok(pwm) => Box::new(pwm),
            Err(err) => {
                println!("Error: {err}");
                exit(1);
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!(
            "Error: driving the speaker pin requires Linux (set \"emulation\": true elsewhere)"
        );
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
    let sensor_name: &'static str = Box::leak(cfg.sensor_name.clone().into_boxed_str());

    let speaker: SharedSpeaker = Arc::new(Mutex::new(SpeakerController::new(
        make_output(&cfg),
        Box::new(MonotonicClock),
    )));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let tick_speaker = speaker.clone();
            let command_speaker = speaker.clone();
            let result = common::ble::run_ble(
                sensor_name,
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || {
                        let mut speaker = tick_speaker.lock().unwrap();
                        speaker.tick();
                        speaker.take_pending().then(|| speaker.state())
                    }),
                    interval: POLL_INTERVAL,
                    on_command: Some(Box::new(move |packet| {
                        let mut speaker = command_speaker.lock().unwrap();
                        if let Err(err) = controller::handle_ble_command(&mut speaker, packet) {
                            println!("BLE command failed: {err}");
                        }
                    })),
                },
            );
            // Leave the speaker silent rather than sounding after exit.
            speaker.lock().unwrap().close();
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
            let speaker = speaker.clone();
            move |send: &mut dyn FnMut(&Payload)| send(&speaker.lock().unwrap().state())
        },
        {
            let speaker = speaker.clone();
            move |message: &str| handle_json_command(&mut speaker.lock().unwrap(), message)
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

    // Playback loop: ends tones on time and publishes every change. Command
    // handlers only mutate the controller, so the app sees a command echo
    // and a tone that ran out the same way.
    {
        let server = server.clone();
        let speaker = speaker.clone();
        std::thread::spawn(move || loop {
            if STOP.load(Ordering::SeqCst) {
                speaker.lock().unwrap().close();
                println!("Stopped.");
                exit(0);
            }
            let payload = {
                let mut speaker = speaker.lock().unwrap();
                speaker.tick();
                speaker.take_pending().then(|| speaker.state())
            };
            if let Some(payload) = payload {
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
