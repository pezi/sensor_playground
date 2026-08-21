//! Playback state for the speaker node — the single source of truth this
//! node publishes — and the command parsers that stage it.
//!
//! Command handlers only stage playback; tick(), driven by the serving
//! loop, is what advances melodies and ends tones on time, and every change
//! marks the state as pending publication.
//!
//! The BLE command parser is only reachable on Linux, where the BLE
//! transport is.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use common::Payload;
use serde_json::json;
use std::time::{Duration, Instant};

/// Accepted tone range; anything else is ignored as noise.
pub const MIN_FREQ_HZ: u32 = 20;
pub const MAX_FREQ_HZ: u32 = 20000;

/// The built-in melody: a little C-major fanfare, (freq Hz, duration ms).
pub const MELODY: [(u32, u64); 7] = [
    (262, 250),
    (330, 250),
    (392, 250),
    (523, 350),
    (392, 250),
    (330, 250),
    (262, 500),
];

/// BLE command opcodes.
pub const CMD_TONE: u8 = 0x01; // 0x01 <freq:u16 BE> <ms:u16 BE>
pub const CMD_MELODY: u8 = 0x02; // play the melody
pub const CMD_STOP: u8 = 0x03; // silence

/// A monotonic clock, injectable so the tests need no timers.
pub trait Clock: Send {
    fn now(&self) -> Instant;
}

/// The real clock.
pub struct MonotonicClock;

impl Clock for MonotonicClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

pub struct SpeakerController {
    output: Box<dyn crate::output::Output>,
    clock: Box<dyn Clock>,
    frequency: u32,
    pending: bool,
    deadline: Option<Instant>,
    remaining: Vec<(u32, u64)>, // remaining melody steps
}

impl SpeakerController {
    pub fn new(mut output: Box<dyn crate::output::Output>, clock: Box<dyn Clock>) -> Self {
        output.play(0);
        Self {
            output,
            clock,
            frequency: 0,
            pending: true, // publish the initial state as soon as we serve
            deadline: None,
            remaining: Vec::new(),
        }
    }

    /// Start one note.
    fn apply(&mut self, frequency: u32, milliseconds: u64) {
        self.frequency = frequency;
        self.deadline = if frequency > 0 {
            Some(self.clock.now() + Duration::from_millis(milliseconds))
        } else {
            None
        };
        self.output.play(frequency);
        // Publish unconditionally: a redundant command from a client that
        // guessed wrong would otherwise never be corrected.
        self.pending = true;
    }

    /// Play one tone, cancelling any melody.
    pub fn play_tone(&mut self, frequency: u32, milliseconds: u64) {
        if !(MIN_FREQ_HZ..=MAX_FREQ_HZ).contains(&frequency) || milliseconds == 0 {
            println!("Ignoring tone {frequency} Hz / {milliseconds} ms");
            return;
        }
        println!("Tone: {frequency} Hz for {milliseconds} ms");
        self.remaining.clear();
        self.apply(frequency, milliseconds);
    }

    /// Start the built-in melody from its first note.
    pub fn play_melody(&mut self) {
        println!("Melody");
        self.remaining = MELODY[1..].to_vec();
        self.apply(MELODY[0].0, MELODY[0].1);
    }

    /// Silence the speaker, whatever it is playing.
    pub fn stop(&mut self) {
        println!("Stop");
        self.remaining.clear();
        self.apply(0, 0);
    }

    /// End a finished tone, or step through the melody.
    pub fn tick(&mut self) {
        match self.deadline {
            Some(deadline) if self.clock.now() >= deadline => {}
            _ => return,
        }
        if self.remaining.is_empty() {
            self.apply(0, 0);
            return;
        }
        let (frequency, milliseconds) = self.remaining.remove(0);
        self.apply(frequency, milliseconds);
    }

    /// True once after each change, clearing the pending flag.
    pub fn take_pending(&mut self) -> bool {
        std::mem::take(&mut self.pending)
    }

    /// The state payload the transports publish.
    pub fn state(&self) -> Payload {
        let mut p = Payload::new();
        p.insert("freq".into(), json!(self.frequency));
        p
    }

    pub fn close(&mut self) {
        self.output.close();
    }
}

/// Execute one JSON command pushed by the app over the WebSocket.
pub fn handle_json_command(speaker: &mut SpeakerController, message: &str) {
    let Ok(command) = serde_json::from_str::<serde_json::Value>(message) else {
        println!("Ignoring malformed command");
        return;
    };
    if command.get("stop") == Some(&json!(true)) {
        speaker.stop();
        return;
    }
    if command.get("melody") == Some(&json!(true)) {
        speaker.play_melody();
        return;
    }
    // Integers only, like the Python node's isinstance(int) check.
    if let Some(tone) = command.get("tone").filter(|t| t.is_object()) {
        if let (Some(frequency), Some(milliseconds)) = (
            tone.get("freq").and_then(|v| v.as_u64()),
            tone.get("ms").and_then(|v| v.as_u64()),
        ) {
            speaker.play_tone(frequency.min(u64::from(u32::MAX)) as u32, milliseconds);
            return;
        }
    }
    println!("Ignoring unknown command");
}

/// Execute one binary command packet written over BLE.
pub fn handle_ble_command(speaker: &mut SpeakerController, packet: &[u8]) -> Result<(), String> {
    let Some(&opcode) = packet.first() else {
        return Err("empty command packet".into());
    };
    match opcode {
        CMD_STOP => speaker.stop(),
        CMD_MELODY => speaker.play_melody(),
        CMD_TONE => {
            if packet.len() < 5 {
                return Err("tone without frequency and duration".into());
            }
            let frequency = u32::from(packet[1]) << 8 | u32::from(packet[2]);
            let milliseconds = u64::from(packet[3]) << 8 | u64::from(packet[4]);
            speaker.play_tone(frequency, milliseconds);
        }
        other => return Err(format!("unknown opcode {other:#04x}")),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A clock the test advances by hand, so no timers are needed to expire
    /// a tone.
    struct TestClock {
        base: Instant,
        offset: Arc<Mutex<Duration>>,
    }

    impl Clock for TestClock {
        fn now(&self) -> Instant {
            self.base + *self.offset.lock().unwrap()
        }
    }

    /// Captures what the controller asked the speaker to sound.
    struct RecordingOutput(Arc<Mutex<Vec<u32>>>);

    impl crate::output::Output for RecordingOutput {
        fn play(&mut self, frequency: u32) {
            self.0.lock().unwrap().push(frequency);
        }
    }

    fn new_speaker() -> (
        SpeakerController,
        Arc<Mutex<Vec<u32>>>,
        Arc<Mutex<Duration>>,
    ) {
        let played = Arc::new(Mutex::new(Vec::new()));
        let offset = Arc::new(Mutex::new(Duration::ZERO));
        let mut speaker = SpeakerController::new(
            Box::new(RecordingOutput(played.clone())),
            Box::new(TestClock {
                base: Instant::now(),
                offset: offset.clone(),
            }),
        );
        speaker.take_pending(); // drop the initial state
        (speaker, played, offset)
    }

    /// The frequency the node would publish.
    fn frequency(speaker: &SpeakerController) -> u64 {
        speaker.state()["freq"].as_u64().unwrap()
    }

    fn advance(offset: &Arc<Mutex<Duration>>, milliseconds: u64) {
        *offset.lock().unwrap() += Duration::from_millis(milliseconds);
    }

    /// An accepted tone sounds, is published, and expires on its own.
    #[test]
    fn a_tone_ends_on_time() {
        let (mut speaker, played, offset) = new_speaker();
        handle_json_command(&mut speaker, r#"{"tone": {"freq": 440, "ms": 400}}"#);
        assert_eq!(frequency(&speaker), 440);
        assert!(speaker.take_pending(), "an accepted tone must be published");
        assert_eq!(played.lock().unwrap().last(), Some(&440));

        advance(&offset, 399);
        speaker.tick();
        assert_eq!(
            frequency(&speaker),
            440,
            "the tone must last its full duration"
        );
        advance(&offset, 1);
        speaker.tick();
        assert_eq!(frequency(&speaker), 0, "the tone must end on time");
        assert!(
            speaker.take_pending(),
            "the end of a tone must be published"
        );
    }

    /// Anything outside the audible range, or malformed, is ignored.
    #[test]
    fn out_of_range_and_malformed_tones_are_ignored() {
        for command in [
            r#"{"tone": {"freq": 19, "ms": 400}}"#,
            r#"{"tone": {"freq": 20001, "ms": 400}}"#,
            r#"{"tone": {"freq": 440, "ms": 0}}"#,
            r#"{"tone": {"freq": 440}}"#,
            r#"{"nonsense": true}"#,
            "not json",
        ] {
            let (mut speaker, _, _) = new_speaker();
            handle_json_command(&mut speaker, command);
            assert_eq!(frequency(&speaker), 0, "{command} must be ignored");
            assert!(!speaker.take_pending(), "{command} must not be published");
        }
    }

    /// The melody walks its notes and falls silent at the end.
    #[test]
    fn the_melody_steps_through_its_notes() {
        let (mut speaker, _, offset) = new_speaker();
        handle_json_command(&mut speaker, r#"{"melody": true}"#);
        assert_eq!(frequency(&speaker), u64::from(MELODY[0].0));
        for (note, _) in &MELODY[1..] {
            advance(&offset, 1000);
            speaker.tick();
            assert_eq!(frequency(&speaker), u64::from(*note));
        }
        advance(&offset, 1000);
        speaker.tick();
        assert_eq!(frequency(&speaker), 0, "the melody must end silent");
        speaker.tick();
        assert_eq!(frequency(&speaker), 0, "silence must not restart anything");
    }

    /// Stop cancels a running melody rather than letting it resume.
    #[test]
    fn stop_cancels_the_melody() {
        let (mut speaker, _, offset) = new_speaker();
        handle_json_command(&mut speaker, r#"{"melody": true}"#);
        handle_json_command(&mut speaker, r#"{"stop": true}"#);
        assert_eq!(frequency(&speaker), 0);
        assert!(speaker.take_pending(), "stop must be published");
        advance(&offset, 1000);
        speaker.tick();
        assert_eq!(frequency(&speaker), 0, "a stopped melody must not resume");
    }

    /// The BLE packets carry the same three actions as the JSON commands.
    #[test]
    fn ble_packets_drive_the_same_actions() {
        let (mut speaker, _, _) = new_speaker();
        // 0x01 <freq:u16 BE> <ms:u16 BE> — 440 Hz for 400 ms.
        handle_ble_command(&mut speaker, &[CMD_TONE, 0x01, 0xB8, 0x01, 0x90]).unwrap();
        assert_eq!(frequency(&speaker), 440);
        handle_ble_command(&mut speaker, &[CMD_STOP]).unwrap();
        assert_eq!(frequency(&speaker), 0);
        handle_ble_command(&mut speaker, &[CMD_MELODY]).unwrap();
        assert_eq!(frequency(&speaker), u64::from(MELODY[0].0));

        assert!(
            handle_ble_command(&mut speaker, &[]).is_err(),
            "empty packet"
        );
        assert!(
            handle_ble_command(&mut speaker, &[0x7F]).is_err(),
            "unknown opcode"
        );
        assert!(
            handle_ble_command(&mut speaker, &[CMD_TONE, 0x01]).is_err(),
            "tone without a duration"
        );
    }
}
