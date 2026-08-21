//! Bit-banged driver for the Sensirion SHT1x two-wire protocol
//! (datasheet V5), ported from the Python node. The bus is fully
//! master-clocked with no minimum speed, so the pace of one GPIO
//! character-device call per clock edge is harmless — the sensor simply
//! waits between edges. The DATA line is driven open-drain style:
//! released (input with pull-up) for a 1, actively pulled LOW for a 0,
//! because the sensor drives the same wire when answering. It is
//! requested once and *reconfigured* in place for each direction flip —
//! the same mode flips the Python node does via lgpio claims.
//!
//! The conversion math is platform-neutral (and unit tested); only the
//! GPIO access is Linux-only.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::time::Duration;

#[cfg(target_os = "linux")]
use gpiocdev::line::{Bias, Value};
#[cfg(target_os = "linux")]
use gpiocdev::request::Config;
#[cfg(target_os = "linux")]
use gpiocdev::Request;
#[cfg(target_os = "linux")]
use std::time::Instant;

// -- SHT1x protocol constants (datasheet V5) ----------------------------------

/// 000 00011: measure temperature (14 bit).
pub const CMD_TEMPERATURE: u8 = 0x03;
/// 000 00101: measure humidity (12 bit).
pub const CMD_HUMIDITY: u8 = 0x05;

/// A 14-bit measurement takes up to 320 ms.
const MEASURE_TIMEOUT: Duration = Duration::from_millis(400);

/// 14-bit temperature at 3.3V supply: T = D1 + T1 * raw.
const D1: f64 = -39.66;

// 12-bit humidity polynomial + temperature compensation.
const C1: f64 = -2.0468;
const C2: f64 = 0.0367;
const C3: f64 = -1.5955e-6;
const T1: f64 = 0.01;
const T2: f64 = 0.00008;

/// Converts a raw 14-bit temperature reading to °C.
pub fn sht1x_temperature(raw: u16) -> f64 {
    D1 + T1 * f64::from(raw)
}

/// Converts a raw 12-bit humidity reading to %RH (compensated).
pub fn sht1x_humidity(raw: u16, temperature: f64) -> f64 {
    let r = f64::from(raw);
    let linear = C1 + C2 * r + C3 * r * r;
    let compensated = (temperature - 25.0) * (T1 + T2 * r) + linear;
    compensated.clamp(0.0, 100.0)
}

// -- Bus ----------------------------------------------------------------------

/// Bit-bangs the two-wire protocol on two GPIO lines: SCK as a plain
/// output, DATA reconfigured between output-low and input-with-pull-up.
#[cfg(target_os = "linux")]
pub struct Sht1xBus {
    sck: Request,
    sck_offset: u32,
    data: Request,
    data_offset: u32,
    data_output_cfg: Config,
    data_input_cfg: Config,
}

#[cfg(target_os = "linux")]
fn gpio_err(e: gpiocdev::Error) -> String {
    e.to_string()
}

#[cfg(target_os = "linux")]
impl Sht1xBus {
    /// Claims the two lines and resets the sensor interface. A wrong
    /// chip/pin fails here instead of on every read.
    pub fn open(chip: &str, data_offset: u32, sck_offset: u32) -> Result<Self, String> {
        let mut sck_builder = Request::builder();
        sck_builder
            .on_chip(chip)
            .with_consumer("sensor-playground-sht11")
            .with_line(sck_offset)
            .as_output(Value::Inactive);
        let sck = sck_builder
            .request()
            .map_err(|e| format!("requesting SCK line {sck_offset} on {chip}: {e}"))?;

        // DATA idles released: the pull-up holds it high and the sensor
        // may drive it low.
        let mut data_builder = Request::builder();
        data_builder
            .on_chip(chip)
            .with_consumer("sensor-playground-sht11")
            .with_line(data_offset)
            .as_input()
            .with_bias(Bias::PullUp);
        let data = data_builder
            .request()
            .map_err(|e| format!("requesting DATA line {data_offset} on {chip}: {e}"))?;

        let mut data_output_cfg = Config::default();
        data_output_cfg
            .with_line(data_offset)
            .as_output(Value::Inactive);
        let mut data_input_cfg = Config::default();
        data_input_cfg
            .with_line(data_offset)
            .as_input()
            .with_bias(Bias::PullUp);

        let bus = Self {
            sck,
            sck_offset,
            data,
            data_offset,
            data_output_cfg,
            data_input_cfg,
        };
        // The sensor needs 11 ms after power-up before the first command.
        std::thread::sleep(Duration::from_millis(20));
        bus.connection_reset()?;
        Ok(bus)
    }

    // -- line helpers: DATA is bidirectional and never driven HIGH --------

    /// Releases DATA (the pull-up makes it HIGH, the sensor may drive it).
    fn data_release(&self) -> Result<(), String> {
        self.data.reconfigure(&self.data_input_cfg).map_err(gpio_err)
    }

    /// Actively pulls DATA LOW.
    fn data_low(&self) -> Result<(), String> {
        self.data
            .reconfigure(&self.data_output_cfg)
            .map_err(gpio_err)
    }

    /// Samples the DATA line.
    fn data_read(&self) -> Result<bool, String> {
        Ok(self.data.value(self.data_offset).map_err(gpio_err)? == Value::Active)
    }

    fn sck_write(&self, level: bool) -> Result<(), String> {
        let value = if level { Value::Active } else { Value::Inactive };
        self.sck.set_value(self.sck_offset, value).map_err(gpio_err)
    }

    fn sck_pulse(&self) -> Result<(), String> {
        self.sck_write(true)?;
        self.sck_write(false)
    }

    // -- protocol ---------------------------------------------------------

    /// DATA falls and rises while SCK is high — the start pattern.
    fn transmission_start(&self) -> Result<(), String> {
        self.data_release()?;
        self.sck_write(false)?;
        self.sck_write(true)?;
        self.data_low()?;
        self.sck_write(false)?;
        self.sck_write(true)?;
        self.data_release()?;
        self.sck_write(false)
    }

    /// Resynchronises the interface: DATA high, nine or more clocks.
    pub fn connection_reset(&self) -> Result<(), String> {
        self.data_release()?;
        self.sck_write(false)?;
        for _ in 0..10 {
            self.sck_pulse()?;
        }
        Ok(())
    }

    /// Sends one command byte; returns false without the sensor's ACK.
    fn send_command(&self, command: u8) -> Result<bool, String> {
        self.transmission_start()?;
        for bit in (0..8).rev() {
            if command & (1 << bit) != 0 {
                self.data_release()?;
            } else {
                self.data_low()?;
            }
            self.sck_pulse()?;
        }
        // ACK: the sensor pulls DATA low during the ninth clock.
        self.data_release()?;
        self.sck_write(true)?;
        let acked = !self.data_read()?;
        self.sck_write(false)?;
        Ok(acked)
    }

    /// Reads one byte; `ack` keeps the transfer going, its absence ends it
    /// (the sensor then skips the CRC byte, which is not used here).
    fn read_byte(&self, ack: bool) -> Result<u8, String> {
        let mut value = 0u8;
        self.data_release()?;
        for bit in (0..8).rev() {
            self.sck_write(true)?;
            if self.data_read()? {
                value |= 1 << bit;
            }
            self.sck_write(false)?;
        }
        if ack {
            self.data_low()?;
        } else {
            self.data_release()?;
        }
        self.sck_pulse()?;
        self.data_release()?;
        Ok(value)
    }

    /// Runs one measurement; returns the raw result.
    pub fn measure(&self, command: u8) -> Result<u16, String> {
        if !self.send_command(command)? {
            return Err(format!("no ACK for command 0x{command:02x}"));
        }
        // The sensor releases DATA while measuring, pulls it low when done.
        let deadline = Instant::now() + MEASURE_TIMEOUT;
        while self.data_read()? {
            if Instant::now() > deadline {
                return Err("measurement timeout".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let msb = self.read_byte(true)?;
        let lsb = self.read_byte(false)?;
        Ok(u16::from(msb) << 8 | u16::from(lsb))
    }
}

// -- Tests --------------------------------------------------------------------

/// The conversion formulas (datasheet V5: 14-bit temperature at 3.3V,
/// 12-bit humidity polynomial + temperature compensation) are checked at
/// exact raw values, including the 0..100 %RH clamp. There is no CRC to
/// test: like the Python node, the driver ends the transfer before the
/// CRC byte.
#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    /// T = -39.66 + 0.01 * raw.
    #[test]
    fn temperature_conversion() {
        for (raw, want) in [(0u16, -39.66), (3966, 0.0), (6566, 26.0)] {
            let got = sht1x_temperature(raw);
            assert!(near(got, want), "temperature({raw}) = {got}, want {want}");
        }
    }

    /// RH = C1 + C2*raw + C3*raw² plus (T - 25)(T1 + T2*raw) compensation,
    /// clamped to the physical 0..100 %RH range.
    #[test]
    fn humidity_conversion() {
        // At 25 °C the compensation term vanishes: the pure polynomial.
        let got = sht1x_humidity(1500, 25.0);
        assert!(near(got, 49.413325), "humidity(1500, 25.0) = {got}");
        // 10 °C above the reference adds 10 * (T1 + T2*1500) = 1.3.
        let got = sht1x_humidity(1500, 35.0);
        assert!(near(got, 50.713325), "humidity(1500, 35.0) = {got}");
    }

    /// The polynomial is negative near raw 0 and >100 at high raw values.
    #[test]
    fn humidity_clamped_to_physical_range() {
        assert_eq!(sht1x_humidity(0, 25.0), 0.0);
        assert_eq!(sht1x_humidity(3500, 25.0), 100.0);
    }
}
