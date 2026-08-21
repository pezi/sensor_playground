//! Minimal 9600-8N1 serial port via termios (Linux) — read side only: the
//! Grove RFID reader is transmit-only, the board never sends it anything.

#![cfg(target_os = "linux")]

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::os::fd::AsRawFd;

pub struct SerialPort {
    f: File,
}

impl SerialPort {
    /// Open `path` at 9600 baud, raw 8N1, with non-blocking reads
    /// (VMIN=0, VTIME=0 — a read returns whatever already arrived, or
    /// nothing; the tag loop polls every 50 ms).
    pub fn open(path: &str) -> std::io::Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOCTTY)
            .open(path)?;
        // SAFETY: plain termios calls on an owned fd.
        unsafe {
            let mut tio: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(f.as_raw_fd(), &mut tio) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            libc::cfmakeraw(&mut tio);
            tio.c_cflag |= libc::CREAD | libc::CLOCAL;
            tio.c_cc[libc::VMIN] = 0;
            tio.c_cc[libc::VTIME] = 0;
            libc::cfsetispeed(&mut tio, libc::B9600);
            libc::cfsetospeed(&mut tio, libc::B9600);
            if libc::tcsetattr(f.as_raw_fd(), libc::TCSANOW, &tio) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self { f })
    }

    /// Drain whatever arrived since the last poll (up to buf.len() bytes).
    pub fn read_available(&mut self, buf: &mut [u8]) -> usize {
        self.f.read(buf).unwrap_or(0)
    }
}

use std::os::unix::fs::OpenOptionsExt;
