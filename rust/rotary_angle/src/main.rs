//! Sensor Playground Sensor Node — Grove Rotary Angle Sensor (Rust, push)
//!
//! Reads a Grove Rotary Angle Sensor — a 10 kOhm potentiometer with 300°
//! of mechanical travel — through the Seeed Grove Base Hat's 12-bit ADC
//! and reports the knob position.
//! https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/
//!
//! Unlike the light sensor (also analog, but polled over HTTPS every few
//! seconds) this is a *push* node: the app draws a needle that tracks the
//! knob, so a reading that is seconds old is useless. The node samples
//! the ADC continuously and sends a message whenever the knob has moved
//! further than the deadband, plus the current position once per client
//! connect:
//!
//!     {"adc": 2048, "adcMax": 4095, "angle": 150.1, "angleMax": 300.0}
//!
//! adcMax travels with every message because the converter's width
//! belongs to the board doing the reading, not to the knob: the Grove
//! Base Hat is 12-bit (0-4095), the Arduino-based hats the Python node
//! also supports ("nano", "grovePlus", both 10-bit) are not implemented
//! in this port. `angle`/`angleMax` carry the same position expressed in
//! degrees, so the app never needs to know the knob's mechanical travel
//! either.
//!
//! The Raspberry Pi has no analog input, so an analog sensor needs an
//! extension hat with an ADC — there is no direct-GPIO option (unlike the
//! digital contact node).
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE each movement arrives as a notify on
//!   the data characteristic; the node takes no commands, so there is no
//!   command characteristic.
//!
//! Set "emulation": true in config.json to generate plausible readings
//! without the sensor hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

use common::{config, now_secs, round1, wifi, ws::WsPushServer, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_millis(40); // 25 Hz — matches the ESP32 sketch's publish ceiling

#[allow(dead_code)]
const HAT_I2C_ADDRESS: u16 = 0x04; // Grove Base Hat (STM32F030 ADC)
#[allow(dead_code)]
const HAT_ADC_BASE: u8 = 0x10; // raw 12-bit value registers, one per channel

/// Mechanical travel of the knob, end to end. 300° for the Grove sensor.
const DEFAULT_ANGLE_MAX: f64 = 300.0;

/// Default deadband as a fraction of full scale: how far the count must
/// move before a new message goes out. ADC noise jitters the reading by a
/// few counts with the knob at rest, which would otherwise flood the
/// link.
const DEFAULT_DEADBAND_RATIO: f64 = 0.006; // ~24 counts of 4095

/// Full-scale ADC count per hat, keyed by hat_type. Only "grove" can be
/// read by this port, but the emulation reports the range the configured
/// hat really would, like the Python node.
fn adc_max_for(hat_type: &str) -> i64 {
    match hat_type {
        // NanoHat Hub / GrovePi+, 10-bit AVR analogRead
        "nano" | "grovePlus" => 1023,
        // Grove Base Hat, 12-bit STM32F030 ADC (and the default)
        _ => 4095,
    }
}

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_hat_type")]
    hat_type: String,
    #[serde(default)]
    pin: u8,
    #[serde(default = "default_i2c_bus")]
    i2c_bus: u8,
    #[serde(default = "default_angle_max")]
    angle_max: f64,
    #[serde(default)]
    deadband: Option<i64>,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_hat_type() -> String {
    "grove".into()
}
fn default_i2c_bus() -> u8 {
    1
}
fn default_angle_max() -> f64 {
    DEFAULT_ANGLE_MAX
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Sensor ------------------------------------------------------------------

/// Reads the knob position as a raw ADC count, clamped to 0..adc_max.
type Reader = Box<dyn FnMut() -> Option<i64> + Send>;

fn make_reader(cfg: &Config) -> (Reader, i64) {
    if cfg.emulation {
        println!("Emulation mode: generating ROTARY readings without hardware");
        // Sweeps slowly from end to end and back, as if someone were
        // turning the knob by hand, so the app's needle has something to
        // follow. The emulation reports the range the configured hat
        // would really report, so the app is exercised against a 10-bit
        // node as easily as a 12-bit one.
        let adc_max = adc_max_for(&cfg.hat_type);
        let reader: Reader = Box::new(move || {
            // A 20-second triangle wave over the full span.
            let phase = (now_secs() % 20.0) / 20.0;
            let fraction = 1.0 - (2.0 * phase - 1.0).abs();
            Some((fraction * adc_max as f64).round() as i64)
        });
        return (reader, adc_max);
    }
    if cfg.hat_type != "grove" {
        println!(
            "Error: hat_type \"{}\" is not supported in the Rust port (only \"grove\"; use the Python node for Arduino-based hats)",
            cfg.hat_type
        );
        exit(1);
    }
    if cfg.pin > 7 {
        println!("Error: invalid channel {} - valid range [0,7]", cfg.pin);
        exit(1);
    }
    println!(
        "Initializing Grove Rotary Angle Sensor on grove hat, channel {}...",
        cfg.pin
    );
    #[cfg(target_os = "linux")]
    {
        let mut dev = match common::i2c::I2CDevice::open(cfg.i2c_bus, HAT_I2C_ADDRESS) {
            Ok(dev) => dev,
            Err(err) => {
                println!("Error: opening /dev/i2c-{}: {err}", cfg.i2c_bus);
                exit(1);
            }
        };
        let adc_max = adc_max_for("grove");
        let reg = HAT_ADC_BASE + cfg.pin;
        let reader: Reader = Box::new(move || {
            // The raw 12-bit ADC value of the channel.
            let value = dev.read_word(reg).ok()? as i64;
            Some(value.clamp(0, adc_max))
        });
        (reader, adc_max)
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!("Error: the Grove Base Hat requires Linux (set \"emulation\": true elsewhere)");
        exit(1);
    }
}

/// Builds the position message. The scale travels with every reading.
fn payload_for(adc: i64, adc_max: i64, angle_max: f64) -> Payload {
    let mut p = Payload::new();
    p.insert("adc".into(), json!(adc));
    p.insert("adcMax".into(), json!(adc_max));
    p.insert(
        "angle".into(),
        json!(round1(adc as f64 / adc_max as f64 * angle_max)),
    );
    p.insert("angleMax".into(), json!(angle_max));
    p
}

/// Scales the deadband to this hat's range, so a 10-bit node is not held
/// to a 12-bit node's precision.
fn default_deadband(adc_max: i64) -> i64 {
    ((adc_max as f64 * DEFAULT_DEADBAND_RATIO).ceil() as i64).max(1)
}

/// Decides whether a fresh count goes on the wire: the first reading and
/// any move past the deadband — and the ends of travel are pinned, so a
/// knob turned fully reports exactly 0 or full scale instead of stopping
/// a deadband short of it.
fn should_publish(published: Option<i64>, adc: i64, adc_max: i64, deadband: i64) -> bool {
    match published {
        None => true,
        Some(prev) => {
            (adc - prev).abs() >= deadband || ((adc == 0 || adc == adc_max) && adc != prev)
        }
    }
}

// -- Sample tick -------------------------------------------------------------

/// One sample step, shared between the Wi-Fi loop and the BLE tick: reads
/// the knob and returns the message to publish when it has moved past the
/// deadband (or hit an end of travel).
struct RotaryTick {
    reader: Arc<Mutex<Reader>>,
    last_adc: Arc<Mutex<Option<i64>>>,
    published: Option<i64>,
    adc_max: i64,
    angle_max: f64,
    deadband: i64,
}

impl RotaryTick {
    fn tick(&mut self) -> Option<Payload> {
        let adc = (self.reader.lock().unwrap())()?;
        if !should_publish(self.published, adc, self.adc_max, self.deadband) {
            return None;
        }
        self.published = Some(adc);
        *self.last_adc.lock().unwrap() = Some(adc);
        let payload = payload_for(adc, self.adc_max, self.angle_max);
        println!("adc: {}/{}  angle: {}", adc, self.adc_max, payload["angle"]);
        Some(payload)
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);

    let (reader, adc_max) = make_reader(&cfg);
    let reader = Arc::new(Mutex::new(reader));
    let angle_max = cfg.angle_max;

    println!("ADC range: 0-{adc_max}, travel {angle_max:.0} deg");

    // A deadband given in counts wins; otherwise scale it to this hat's
    // range.
    let deadband = cfg.deadband.unwrap_or_else(|| default_deadband(adc_max));

    // Latest published count, shared with newly connected clients.
    let last_adc: Arc<Mutex<Option<i64>>> = Arc::new(Mutex::new(None));

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let mut tick = RotaryTick {
                reader,
                last_adc,
                published: None,
                adc_max,
                angle_max,
                deadband,
            };
            if let Err(err) = common::ble::run_ble(
                "ROTARY",
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick.tick()),
                    interval: POLL_INTERVAL,
                    on_command: None,
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
            // Send the current position to a client right after it
            // connects.
            let reader = reader.clone();
            let last_adc = last_adc.clone();
            move |send: &mut dyn FnMut(&Payload)| {
                let adc = *last_adc.lock().unwrap();
                let adc = match adc {
                    Some(adc) => Some(adc),
                    None => (reader.lock().unwrap())(),
                };
                if let Some(adc) = adc {
                    send(&payload_for(adc, adc_max, angle_max));
                }
            }
        },
        |_message: &str| {}, // this node takes no commands
    ));

    {
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            if let Err(err) = wifi::run_discovery_listener("ROTARY", &hostname, wifi::WS_PORT, None)
            {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    // Sample loop: publishes whenever the knob moves past the deadband.
    {
        let server = server.clone();
        std::thread::spawn(move || {
            let mut tick = RotaryTick {
                reader,
                last_adc,
                published: None,
                adc_max,
                angle_max,
                deadband,
            };
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

// -- Tests -------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_reading_always_publishes() {
        assert!(should_publish(None, 2048, 4095, 25));
    }

    #[test]
    fn deadband_filters_jitter() {
        assert!(!should_publish(Some(2048), 2058, 4095, 25));
        assert!(should_publish(Some(2048), 2073, 4095, 25));
        assert!(should_publish(Some(2048), 2020, 4095, 25));
    }

    #[test]
    fn ends_of_travel_are_pinned() {
        assert!(should_publish(Some(10), 0, 4095, 25));
        assert!(should_publish(Some(4090), 4095, 4095, 25));
        // A knob resting at an end stays quiet.
        assert!(!should_publish(Some(0), 0, 4095, 25));
        assert!(!should_publish(Some(4095), 4095, 4095, 25));
    }

    #[test]
    fn default_deadband_scales_with_the_hat() {
        // ~0.6 % of full scale, rounded up, never below one count.
        assert_eq!(default_deadband(4095), 25);
        assert_eq!(default_deadband(1023), 7);
        assert_eq!(default_deadband(1), 1);
    }

    #[test]
    fn payload_carries_the_scale() {
        let p = payload_for(2048, 4095, 300.0);
        assert_eq!(p["adc"], json!(2048));
        assert_eq!(p["adcMax"], json!(4095));
        // 2048 / 4095 * 300 = 150.04... -> rounded to one decimal
        assert_eq!(p["angle"], json!(150.0));
        assert_eq!(p["angleMax"], json!(300.0));
    }

    #[test]
    fn angle_maps_the_ends_exactly() {
        assert_eq!(payload_for(0, 4095, 300.0)["angle"], json!(0.0));
        assert_eq!(payload_for(4095, 4095, 300.0)["angle"], json!(300.0));
        assert_eq!(payload_for(1023, 1023, 300.0)["angle"], json!(300.0));
    }

    #[test]
    fn hat_ranges() {
        assert_eq!(adc_max_for("grove"), 4095);
        assert_eq!(adc_max_for("nano"), 1023);
        assert_eq!(adc_max_for("grovePlus"), 1023);
        assert_eq!(adc_max_for("unknown"), 4095);
    }
}
