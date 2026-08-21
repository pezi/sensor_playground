//! TM1637 segment encoding plus the display abstraction the clock drives:
//! a real panel on the bit-banged two-wire bus (see gpio.rs, Linux only)
//! or an emulated one that just prints. The encoding itself is pure math
//! and unit-tested below.
//!
//! The encoding is only *driven* on Linux (the bit-banged panel); off
//! Linux it is still built and tested, hence the dead-code allowance.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

/// Segment patterns for the digits 0-9, gfedcba bit order.
pub const SEGMENT_DIGITS: [u8; 10] = [
    0x3F, 0x06, 0x5B, 0x4F, 0x66, 0x6D, 0x7D, 0x07, 0x7F, 0x6F,
];
/// A lone middle segment (g), the "no time yet" placeholder digit.
pub const SEGMENT_DASH: u8 = 0x40;
/// The Grove module wires the colon to bit 7 of the second digit.
pub const SEGMENT_COLON: u8 = 0x80;

/// The four digit patterns for the panel: dashes when no time is set,
/// otherwise HH:MM with the colon on the second digit while it blinks on.
pub fn segments_for_time(time: Option<(u8, u8)>, colon_on: bool) -> [u8; 4] {
    let Some((hour, minute)) = time else {
        return [SEGMENT_DASH; 4];
    };
    let mut segments = [
        SEGMENT_DIGITS[(hour / 10) as usize],
        SEGMENT_DIGITS[(hour % 10) as usize],
        SEGMENT_DIGITS[(minute / 10) as usize],
        SEGMENT_DIGITS[(minute % 10) as usize],
    ];
    if colon_on {
        segments[1] |= SEGMENT_COLON;
    }
    segments
}

/// What the clock controller writes to — a panel or the console.
pub trait Display: Send {
    /// Writes the time (or dashes when `time` is None) to the panel.
    fn render(&mut self, time: Option<(u8, u8)>, colon_on: bool, brightness: u8);
    fn close(&mut self) {}
}

/// Prints the displayed state instead of driving hardware. Prints on
/// time/brightness changes and minute rollovers only — the colon blink
/// would flood the console at 1 Hz.
pub struct EmulatedDisplay {
    last_printed: Option<(Option<(u8, u8)>, u8)>,
}

impl EmulatedDisplay {
    pub fn new() -> Self {
        Self { last_printed: None }
    }
}

impl Display for EmulatedDisplay {
    fn render(&mut self, time: Option<(u8, u8)>, _colon_on: bool, brightness: u8) {
        let shown = (time, brightness);
        if self.last_printed == Some(shown) {
            return;
        }
        self.last_printed = Some(shown);
        let text = match time {
            Some((hour, minute)) => format!("{hour:02}:{minute:02}"),
            None => "--:--".to_string(),
        };
        println!("[emulation] display [{text}] brightness={brightness}");
    }
}

// -- Tests --------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The segment patterns the panel receives. Golden values from the
    /// Python node's SEGMENT_DIGITS table; the colon is bit 7 of the
    /// second digit and only ever set while the blink is on.
    #[test]
    fn segments_for_time_encodes_digits() {
        assert_eq!(segments_for_time(None, false), [0x40, 0x40, 0x40, 0x40]);
        // A lit colon must not leak into the placeholder frame.
        assert_eq!(segments_for_time(None, true), [0x40, 0x40, 0x40, 0x40]);
        assert_eq!(
            segments_for_time(Some((12, 34)), false),
            [0x06, 0x5B, 0x4F, 0x66]
        );
        assert_eq!(
            segments_for_time(Some((12, 34)), true),
            [0x06, 0x5B | 0x80, 0x4F, 0x66]
        );
        assert_eq!(
            segments_for_time(Some((0, 0)), true),
            [0x3F, 0x3F | 0x80, 0x3F, 0x3F]
        );
        assert_eq!(
            segments_for_time(Some((23, 59)), false),
            [0x5B, 0x4F, 0x6D, 0x6F]
        );
        assert_eq!(
            segments_for_time(Some((9, 7)), false),
            [0x3F, 0x6F, 0x3F, 0x07]
        );
    }

    /// Every digit pattern lights only the seven segments a-g (bit 7 is
    /// the colon and belongs to no digit), and no two digits share one.
    #[test]
    fn digit_patterns_are_distinct_seven_segment() {
        for (digit, pattern) in SEGMENT_DIGITS.iter().enumerate() {
            assert_eq!(pattern & SEGMENT_COLON, 0, "digit {digit} sets the colon bit");
            let duplicates = SEGMENT_DIGITS.iter().filter(|p| *p == pattern).count();
            assert_eq!(duplicates, 1, "digit {digit} shares its pattern");
        }
        assert_eq!(SEGMENT_DASH & SEGMENT_COLON, 0);
    }

    /// The emulated panel prints once per distinct time/brightness pair —
    /// the colon blink alone never prints.
    #[test]
    fn emulated_display_deduplicates_blinks() {
        let mut display = EmulatedDisplay::new();
        display.render(Some((12, 34)), true, 3);
        assert_eq!(display.last_printed, Some((Some((12, 34)), 3)));
        display.render(Some((12, 34)), false, 3);
        assert_eq!(display.last_printed, Some((Some((12, 34)), 3)));
        display.render(Some((12, 35)), false, 3);
        assert_eq!(display.last_printed, Some((Some((12, 35)), 3)));
    }
}
