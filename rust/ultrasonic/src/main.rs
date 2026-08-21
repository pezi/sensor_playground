//! Sensor Playground Sensor Node — Grove Ultrasonic Ranger (Rust)
//!
//! Implements the *push* variant of the Sensor Playground Sensor Interface
//! on single-board computers (Raspberry Pi & co.) with a Grove Ultrasonic
//! Ranger (40 kHz sonar, 2 cm - 3.5 m). Like the VL53L0X node it measures
//! continuously and pushes one JSON message ({"distance": <mm>}, null when
//! no echo returns) whenever the distance changes, or at least once per
//! second as a heartbeat.
//!
//! The Grove ranger uses a single SIG pin for both trigger and echo —
//! unlike the common HC-SR04 with separate TRIG/ECHO pins. One measurement:
//! drive SIG with a trigger pulse, switch the pin to input, and time the
//! echo pulse the module answers with; the pulse width divided by twice the
//! speed of sound is the distance.
//!
//! The echo is timed via kernel-timestamped GPIO edge events (gpiocdev) —
//! a userspace polling loop would add milliseconds of jitter, and one
//! millisecond of pulse error is 17 cm of distance error. (The trigger
//! pulse only has a minimum width, so thread::sleep's overshoot is
//! harmless there.) There is no extension-hat option: the Arduino-based
//! hats are polled over I2C and cannot time the echo.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE each measurement arrives as a notify
//!   on the data characteristic.
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
use std::sync::Arc;
use std::time::{Duration, Instant};

// -- Publish policy -----------------------------------------------------------

const MEASURE_INTERVAL: Duration = Duration::from_millis(100);
const HEARTBEAT: Duration = Duration::from_secs(1);
/// Larger than the VL53L0X's delta because a sonar reading jitters a little.
const MIN_DELTA_MM: i64 = 5;

// -- Measurement --------------------------------------------------------------

/// Sound travels 0.343 mm/µs = 343 mm per million ns; the echo pulse
/// covers the distance twice.
const MM_PER_NS: f64 = 0.343e-3 / 2.0;

/// The echo of a 3.5 m target takes ~20 ms; give up shortly after that.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const ECHO_TIMEOUT: Duration = Duration::from_millis(60);

/// Trigger pulse: >=10 µs high. thread::sleep overshoots, which the
/// module tolerates — the pulse only has a minimum width.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const TRIGGER_PULSE: Duration = Duration::from_micros(20);

const MIN_RANGE_MM: i64 = 20;
const MAX_RANGE_MM: i64 = 3500;

/// Converts an echo pulse width [ns] to a distance in mm, or None when
/// the result is outside the ranger's 2 cm - 3.5 m range.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn pulse_to_mm(pulse_ns: u64) -> Option<i64> {
    let mm = (pulse_ns as f64 * MM_PER_NS).round() as i64;
    if !(MIN_RANGE_MM..=MAX_RANGE_MM).contains(&mm) {
        return None;
    }
    Some(mm)
}

#[derive(Deserialize)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_sig_pin")]
    sig_pin: u32,
    #[serde(default)]
    gpio_chip: u32,
    #[serde(default = "default_sensor_name")]
    sensor_name: String,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_sig_pin() -> u32 {
    4
}
fn default_sensor_name() -> String {
    "ULTRASONIC".into()
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Hardware ----------------------------------------------------------------

type ReadFn = Box<dyn FnMut() -> Result<Option<i64>, String> + Send>;

/// Generates plausible ranger readings without hardware: a target
/// sweeping back and forth between 200 and 2000 mm (20 s period),
/// occasionally leaving the measuring range.
fn emulated_read() -> Option<i64> {
    if fastrand::f64() < 0.02 {
        return None;
    }
    let phase = (common::now_secs() % 20.0) / 20.0;
    Some((200.0 + 1800.0 * (1.0 - (2.0 * phase - 1.0).abs())).round() as i64)
}

fn make_sensor(cfg: &Config) -> ReadFn {
    if cfg.emulation {
        return Box::new(|| Ok(emulated_read()));
    }
    #[cfg(target_os = "linux")]
    {
        let chip = format!("/dev/gpiochip{}", cfg.gpio_chip);
        match gpio::SigLine::open(&chip, cfg.sig_pin) {
            Ok(line) => Box::new(move || line.read_mm()),
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

// -- Measure tick -------------------------------------------------------------

/// The push policy: the first reading, an echo/no-echo flip, or a change
/// of at least MIN_DELTA_MM publish immediately (heartbeat aside).
fn should_publish(ever_published: bool, last_sent: Option<i64>, mm: Option<i64>) -> bool {
    !ever_published
        || mm.is_none() != last_sent.is_none()
        || matches!((mm, last_sent), (Some(a), Some(b)) if (a - b).abs() >= MIN_DELTA_MM)
}

fn distance_payload(mm: Option<i64>) -> Payload {
    let mut p = Payload::new();
    p.insert("distance".into(), json!(mm));
    p
}

/// One measure step at MEASURE_INTERVAL: read the ranger and decide
/// whether to publish — on change, echo flip, or heartbeat. Both
/// transports funnel through this.
struct MeasureTick {
    read: ReadFn,
    last_sent: Option<i64>,
    ever_published: bool,
    last_publish: Instant,
}

impl MeasureTick {
    fn new(read: ReadFn) -> Self {
        Self {
            read,
            last_sent: None,
            ever_published: false,
            last_publish: Instant::now(),
        }
    }

    fn tick(&mut self) -> Option<Payload> {
        let mm = match (self.read)() {
            Ok(mm) => mm,
            Err(err) => {
                println!("GPIO read failed: {err}");
                return None;
            }
        };
        if should_publish(self.ever_published, self.last_sent, mm)
            || self.last_publish.elapsed() >= HEARTBEAT
        {
            self.ever_published = true;
            self.last_sent = mm;
            self.last_publish = Instant::now();
            return Some(distance_payload(mm));
        }
        None
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);
    let sensor_name: &'static str = Box::leak(cfg.sensor_name.clone().into_boxed_str());

    if cfg.emulation {
        println!("Emulation mode: generating ranger readings without hardware");
    } else {
        println!("Initializing ultrasonic ranger on GPIO {}...", cfg.sig_pin);
    }
    let read = make_sensor(&cfg);

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let mut tick = MeasureTick::new(read);
            if let Err(err) = common::ble::run_ble(
                sensor_name,
                cfg.api_key,
                common::ble::BleRole::Actuator {
                    tick: Box::new(move || tick.tick()),
                    interval: MEASURE_INTERVAL,
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
        |_send: &mut dyn FnMut(&Payload)| {},
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

    // Measure loop: measures continuously and publishes on change, echo
    // flip, or heartbeat.
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

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The pulse-width -> distance conversion: sound travels 0.343 mm/µs
    /// and the echo pulse covers the distance twice, so 1 mm is ~5831 ns
    /// of pulse. Golden values from the Python node's formula.
    #[test]
    fn pulse_to_mm_matches_python_formula() {
        assert_eq!(pulse_to_mm(5_830_904), Some(1000)); // 1 m target
        assert_eq!(pulse_to_mm(116_618), Some(20)); // exactly the 2 cm minimum
        assert_eq!(pulse_to_mm(20_408_163), Some(3500)); // exactly the 3.5 m maximum
        assert_eq!(pulse_to_mm(58_309), None); // 10 mm — below the minimum range
        assert_eq!(pulse_to_mm(25_000_000), None); // ~4.3 m — beyond the maximum range
    }

    /// The push policy: the first reading, an echo/no-echo flip, or a
    /// change of at least MIN_DELTA_MM publish immediately (the heartbeat
    /// is time-based and handled by the tick).
    #[test]
    fn publish_policy() {
        assert!(should_publish(false, None, Some(500))); // first reading
        assert!(should_publish(false, None, None)); // first reading, no echo
        assert!(!should_publish(true, Some(500), Some(500))); // unchanged
        assert!(!should_publish(true, Some(500), Some(504))); // below delta
        assert!(should_publish(true, Some(500), Some(505))); // at delta
        assert!(should_publish(true, Some(500), Some(495))); // at delta downward
        assert!(should_publish(true, Some(500), None)); // echo lost
        assert!(should_publish(true, None, Some(500))); // echo regained
        assert!(!should_publish(true, None, None)); // still no echo
    }
}
