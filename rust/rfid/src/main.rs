//! Sensor Playground Sensor Node — Grove 125KHz RFID Reader (Rust)
//!
//! Implements the *push* variant of the Sensor Playground Sensor
//! Interface on single-board computers (Raspberry Pi & co.) with a Grove
//! 125KHz RFID Reader. The node reads RDM630-style frames from a
//! 9600-baud UART and pushes one JSON message ({"tag": "0F0024ADAB"})
//! per scanned EM4100 tag.
//!
//! Frame format (reader TX, jumper on UART mode — not Wiegand):
//!     STX 0x02 | 10 ASCII-hex data chars | 2 ASCII-hex checksum chars | ETX 0x03
//! The checksum byte is the XOR of the five data bytes. The reader
//! repeats the frame while a tag is held near the antenna, so the node
//! suppresses repeats of the same tag for REPEAT_SUPPRESS.
//!
//! - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
//!   (default), or
//! - BLE GATT server ("transport": "ble" in config.json), like the ESP32
//!   sketch (Linux only). Over BLE each scan arrives as a notify on the
//!   data characteristic; the node takes no commands.
//!
//! Set "emulation": true in config.json to generate plausible scans
//! without the reader hardware (works with both transports).
//!
//! Usage:
//!     cp config.example.json config.json   # edit with your settings
//!     cargo run --release

mod serial;

use common::{config, uniform, wifi, ws::WsPushServer, Payload};
use serde::Deserialize;
use serde_json::json;
use std::process::exit;
use std::sync::Arc;
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Suppress repeats of the same tag while it is held near the antenna.
const REPEAT_SUPPRESS: Duration = Duration::from_secs(2);

// The parser is only exercised by the Linux serial reader (and the tests);
// the emulation path publishes tags directly.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const STX: u8 = 0x02;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const ETX: u8 = 0x03;
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
const FRAME_HEX_CHARS: usize = 12; // 10 data chars + 2 checksum chars

#[derive(Deserialize)]
struct Config {
    api_key: String,
    #[serde(default)]
    hostname: String,
    #[serde(default = "default_serial_port")]
    serial_port: String,
    #[serde(default = "default_transport")]
    transport: String,
    #[serde(default)]
    emulation: bool,
}

fn default_serial_port() -> String {
    "/dev/serial0".into()
}
fn default_transport() -> String {
    "wifi".into()
}

// -- Frame parsing -----------------------------------------------------------

/// Byte-wise state machine for RDM630-style frames.
#[derive(Default)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
struct RdmParser {
    frame: Option<Vec<u8>>, // None = waiting for STX, else collected hex chars
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
impl RdmParser {
    /// Advance the state machine by one byte; returns a validated
    /// 10-char hex tag whenever a byte completes a frame.
    fn feed(&mut self, byte: u8) -> Option<String> {
        if byte == STX {
            self.frame = Some(Vec::new()); // resync, also on a second STX
            return None;
        }
        self.frame.as_ref()?; // noise outside a frame
        if byte == ETX {
            let frame = self.frame.take().unwrap();
            if frame.len() == FRAME_HEX_CHARS && checksum_ok(&frame) {
                return Some(String::from_utf8_lossy(&frame[..10]).to_uppercase());
            }
            return None;
        }
        let frame = self.frame.as_mut().unwrap();
        if byte.is_ascii_hexdigit() {
            frame.push(byte);
            if frame.len() > FRAME_HEX_CHARS {
                self.frame = None; // overflow: wait for the next STX
            }
        } else {
            self.frame = None; // non-hex noise mid-frame
        }
        None
    }
}

/// XOR of the five data bytes must equal the checksum byte.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn checksum_ok(frame: &[u8]) -> bool {
    let mut values = [0u8; FRAME_HEX_CHARS / 2];
    for (i, value) in values.iter_mut().enumerate() {
        let pair = std::str::from_utf8(&frame[i * 2..i * 2 + 2]).ok();
        match pair.and_then(|p| u8::from_str_radix(p, 16).ok()) {
            Some(v) => *value = v,
            None => return false,
        }
    }
    values[..5].iter().fold(0, |acc, v| acc ^ v) == values[5]
}

// -- Readers -----------------------------------------------------------------

type TagReader = Box<dyn FnMut() -> Option<String> + Send>;

fn make_reader(cfg: &Config) -> TagReader {
    if cfg.emulation {
        println!("Emulation mode: generating RFID scans without hardware");
        // One tag from a small fixed pool every four to eight seconds;
        // polls in between return no tag.
        const TAGS: [&str; 4] = ["0F0024ADAB", "0A0031B2C4", "03004F19AA", "1000C0FFEE"];
        let mut next_at = Instant::now() + Duration::from_secs_f64(uniform(4.0, 8.0));
        return Box::new(move || {
            if Instant::now() < next_at {
                return None;
            }
            next_at = Instant::now() + Duration::from_secs_f64(uniform(4.0, 8.0));
            Some(TAGS[fastrand::usize(..TAGS.len())].to_string())
        });
    }
    println!("Opening RFID reader on {}...", cfg.serial_port);
    #[cfg(target_os = "linux")]
    {
        let mut port = match serial::SerialPort::open(&cfg.serial_port) {
            Ok(port) => port,
            Err(err) => {
                println!("Error: opening {}: {err}", cfg.serial_port);
                exit(1);
            }
        };
        let mut parser = RdmParser::default();
        // Drain whatever arrived since the last poll; the last complete
        // frame in the batch wins.
        Box::new(move || {
            let mut buf = [0u8; 64];
            let n = port.read_available(&mut buf);
            let mut result = None;
            for &byte in &buf[..n] {
                if let Some(tag) = parser.feed(byte) {
                    result = Some(tag);
                }
            }
            result
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        println!(
            "Error: the RFID reader serial port requires Linux (set \"emulation\": true elsewhere)"
        );
        exit(1);
    }
}

// -- Tag tick ----------------------------------------------------------------

/// One reader-poll step; returns {"tag": ...} whenever a scan should be
/// published (repeats of the same tag are suppressed while it is held
/// near the antenna). Both transports funnel through this.
struct TagTick {
    read_tag: TagReader,
    last_tag: Option<String>,
    last_at: Option<Instant>,
}

impl TagTick {
    fn new(read_tag: TagReader) -> Self {
        Self {
            read_tag,
            last_tag: None,
            last_at: None,
        }
    }

    fn tick(&mut self) -> Option<Payload> {
        let tag = (self.read_tag)()?;
        if self.last_tag.as_deref() == Some(tag.as_str())
            && self
                .last_at
                .is_some_and(|at| at.elapsed() < REPEAT_SUPPRESS)
        {
            return None;
        }
        println!("Tag: {tag}");
        let mut p = Payload::new();
        p.insert("tag".into(), json!(tag));
        self.last_tag = Some(tag);
        self.last_at = Some(Instant::now());
        Some(p)
    }
}

// -- Main --------------------------------------------------------------------

fn main() {
    let cfg: Config = config::load_config();
    config::require_api_key(&cfg.api_key);
    let hostname = config::hostname_or(&cfg.hostname);

    let reader = make_reader(&cfg);

    if cfg.transport == "ble" {
        #[cfg(target_os = "linux")]
        {
            let mut tick = TagTick::new(reader);
            if let Err(err) = common::ble::run_ble(
                "RFID",
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

    let server = Arc::new(WsPushServer::new(cfg.api_key, |_send| {}, |_message| {}));

    {
        let hostname = hostname.clone();
        std::thread::spawn(move || {
            if let Err(err) = wifi::run_discovery_listener("RFID", &hostname, wifi::WS_PORT, None) {
                println!("UDP discovery failed: {err}");
            }
        });
    }

    // Tag loop: drains the reader and pushes each scan.
    {
        let server = server.clone();
        std::thread::spawn(move || {
            let mut tick = TagTick::new(reader);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(parser: &mut RdmParser, bytes: &[u8]) -> Vec<String> {
        bytes.iter().filter_map(|&b| parser.feed(b)).collect()
    }

    /// Wrap 12 hex chars in STX/ETX.
    fn frame(chars: &str) -> Vec<u8> {
        let mut f = vec![STX];
        f.extend_from_slice(chars.as_bytes());
        f.push(ETX);
        f
    }

    #[test]
    fn parses_valid_frame() {
        let mut p = RdmParser::default();
        assert_eq!(feed_all(&mut p, &frame("0F0024ADAB2D")), ["0F0024ADAB"]);
    }

    #[test]
    fn accepts_lowercase_hex() {
        let mut p = RdmParser::default();
        assert_eq!(feed_all(&mut p, &frame("0f0024adab2d")), ["0F0024ADAB"]);
    }

    #[test]
    fn rejects_bad_checksum() {
        let mut p = RdmParser::default();
        assert!(feed_all(&mut p, &frame("0F0024ADAB2C")).is_empty());
    }

    #[test]
    fn rejects_short_frame() {
        let mut p = RdmParser::default();
        assert!(feed_all(&mut p, &frame("0F0024ADAB")).is_empty());
    }

    #[test]
    fn discards_overflow() {
        let mut p = RdmParser::default();
        assert!(feed_all(&mut p, &frame("0F0024ADAB2D00")).is_empty());
    }

    #[test]
    fn resyncs_on_second_stx() {
        let mut p = RdmParser::default();
        let mut bytes = vec![STX, b'0', b'F', b'0', b'0'];
        bytes.extend(frame("0F0024ADAB2D"));
        assert_eq!(feed_all(&mut p, &bytes), ["0F0024ADAB"]);
    }

    #[test]
    fn ignores_noise_and_recovers() {
        let mut p = RdmParser::default();
        // Noise outside a frame is ignored (including a stray ETX)...
        assert!(feed_all(&mut p, &[0x00, b'A', ETX]).is_empty());
        // ...non-hex noise mid-frame aborts the frame, and parsing
        // recovers on the next one.
        let mut bytes = vec![STX, b'0', b'F', b'$'];
        bytes.extend(frame("1000C0FFEEC1"));
        assert_eq!(feed_all(&mut p, &bytes), ["1000C0FFEE"]);
    }

    #[test]
    fn checksum_matches_tag_pool() {
        for valid in [
            "0F0024ADAB2D",
            "0A0031B2C44D",
            "03004F19AAFF",
            "1000C0FFEEC1",
        ] {
            assert!(checksum_ok(valid.as_bytes()), "checksum_ok({valid})");
        }
        assert!(!checksum_ok(b"0F0024ADAB2C"));
    }
}
