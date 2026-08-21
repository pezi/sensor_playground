//! M24LR64E-R user-memory access over /dev/i2c-N (Linux) — the I2C side of
//! the Grove NFC Tag's dual-interface EEPROM (16-bit memory addressing).
//!
//! The common crate's I2CDevice speaks 8-bit register maps, so this tag
//! gets its own minimal wrapper: an I2C_SLAVE ioctl, then a two-byte
//! address write followed by a sequential read per chunk. The Python node
//! reads the same chunks in one combined i2c_rdwr transaction; the
//! M24LR64's address pointer survives the stop condition in between, so
//! separate write/read transactions return the same bytes.

#![cfg(target_os = "linux")]

use crate::ndef::{parse_ndef_area, Content, SCAN_LENGTH};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;

const I2C_SLAVE: libc::c_ulong = 0x0703;

/// Bytes per I2C transaction, safe on every adapter.
const CHUNK: usize = 32;

/// Reads the M24LR64E-R user memory over I2C.
pub struct M24lr64Tag {
    f: File,
}

impl M24lr64Tag {
    /// Opens the tag and reads it once, failing fast on a wiring/address
    /// problem (like the Python node's constructor).
    pub fn open(bus: u8, addr: u16) -> std::io::Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .open(format!("/dev/i2c-{bus}"))?;
        // SAFETY: a plain ioctl on an owned fd.
        let rc = unsafe { libc::ioctl(f.as_raw_fd(), I2C_SLAVE, addr as libc::c_ulong) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut tag = Self { f };
        tag.read_content()?;
        Ok(tag)
    }

    /// Scans the NDEF area and returns the parsed content.
    pub fn read_content(&mut self) -> std::io::Result<Content> {
        let mut data = vec![0u8; SCAN_LENGTH];
        for offset in (0..SCAN_LENGTH).step_by(CHUNK) {
            self.f
                .write_all(&[(offset >> 8) as u8, (offset & 0xFF) as u8])?;
            self.f.read_exact(&mut data[offset..offset + CHUNK])?;
        }
        Ok(parse_ndef_area(&data))
    }
}
