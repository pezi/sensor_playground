//! Minimal 9600-8N1 serial port via termios (Linux) — just enough for the
//! CozIR's ASCII protocol, without a serial-port dependency.

#![cfg(target_os = "linux")]

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;

pub struct SerialPort {
    f: File,
}

impl SerialPort {
    /// Open `path` at 9600 baud, raw 8N1, with a 1 s read timeout
    /// (VTIME=10, VMIN=0 — a read returns what arrived, or nothing).
    pub fn open(path: &str) -> std::io::Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
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
            tio.c_cc[libc::VTIME] = 10; // deciseconds
            libc::cfsetispeed(&mut tio, libc::B9600);
            libc::cfsetospeed(&mut tio, libc::B9600);
            if libc::tcsetattr(f.as_raw_fd(), libc::TCSANOW, &tio) != 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(Self { f })
    }

    pub fn write_all(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.f.write_all(data)
    }

    /// Discard everything already received but not yet read.
    pub fn flush_input(&mut self) {
        unsafe {
            libc::tcflush(self.f.as_raw_fd(), libc::TCIFLUSH);
        }
    }

    /// Read until a newline, the byte limit, or the port timeout.
    pub fn read_line(&mut self, limit: usize) -> String {
        let mut line = Vec::with_capacity(limit);
        let mut byte = [0u8; 1];
        while line.len() < limit {
            match self.f.read(&mut byte) {
                Ok(1) => {
                    line.push(byte[0]);
                    if byte[0] == b'\n' {
                        break;
                    }
                }
                _ => break, // timeout
            }
        }
        String::from_utf8_lossy(&line).into_owned()
    }
}

use std::os::unix::fs::OpenOptionsExt;
