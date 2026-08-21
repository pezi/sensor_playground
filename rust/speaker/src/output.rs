//! Outputs for the speaker node: a real GPIO line driven with a square
//! wave and an emulated one that just prints.
//!
//! The Grove Speaker is a small amplified loudspeaker on a plain digital
//! pin, so a tone is a 50%-duty square wave of the desired pitch. The
//! Python node gets that from lgpio's tx_pwm, which is itself a software
//! PWM thread — this port does the same thing directly: one thread toggles
//! the line and sleeps half a period, and it is restarted (not signalled)
//! on every pitch change so the timing loop stays a tight two-liner.
//!
//! Timing comes from the scheduler, so the pitch is only as steady as the
//! system's sleep granularity; expect audible jitter under load. That is
//! good enough for a beeper — it is not a music synthesiser.

#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::thread::JoinHandle;
#[cfg(target_os = "linux")]
use std::time::Duration;

/// Drives the speaker at a frequency, or silences it at 0 Hz.
pub trait Output: Send {
    fn play(&mut self, frequency: u32);
    fn close(&mut self) {
        self.play(0);
    }
}

/// Prints the sounding state instead of driving hardware.
pub struct EmulatedOutput;

impl Output for EmulatedOutput {
    fn play(&mut self, frequency: u32) {
        if frequency > 0 {
            println!("[emulation] speaker {frequency} Hz");
        } else {
            println!("[emulation] speaker silent");
        }
    }
}

/// Drives the speaker with a 50%-duty square wave on a GPIO
/// character-device line (/dev/gpiochipN — /sys/class/gpio is gone in
/// Debian 13).
#[cfg(target_os = "linux")]
pub struct PwmOutput {
    line: Arc<crate::gpio::OutputLine>,
    stop: Option<Arc<AtomicBool>>,
    worker: Option<JoinHandle<()>>,
}

#[cfg(target_os = "linux")]
impl PwmOutput {
    /// Claim the speaker pin as an output, silent.
    pub fn open(chip: &str, pin: u32) -> Result<Self, String> {
        Ok(Self {
            line: Arc::new(crate::gpio::OutputLine::open(chip, pin, false)?),
            stop: None,
            worker: None,
        })
    }

    /// Stop the toggling thread and leave the pin low.
    fn silence(&mut self) {
        if let Some(stop) = self.stop.take() {
            stop.store(true, Ordering::SeqCst);
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.line.set(false);
    }
}

#[cfg(target_os = "linux")]
impl Output for PwmOutput {
    fn play(&mut self, frequency: u32) {
        self.silence();
        if frequency == 0 {
            return;
        }
        // Half a period per level, so one full cycle is 1/frequency seconds.
        let half_period = Duration::from_secs_f64(1.0 / f64::from(frequency) / 2.0);
        let stop = Arc::new(AtomicBool::new(false));
        let line = self.line.clone();
        let worker_stop = stop.clone();
        self.stop = Some(stop);
        self.worker = Some(std::thread::spawn(move || {
            let mut level = false;
            while !worker_stop.load(Ordering::SeqCst) {
                level = !level;
                line.set(level);
                std::thread::sleep(half_period);
            }
            line.set(false);
        }));
    }
}
