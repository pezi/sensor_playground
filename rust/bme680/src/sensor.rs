//! Sensor layer: real BME680 (Linux only) and an emulated variant, both
//! feeding the same IAQ calculation.
//! Mirrors python/bme680/sensor_node.py.

const GAS_BURN_IN: usize = 50;
const HUMIDITY_BASELINE: f64 = 40.0;
const HUMIDITY_WEIGHT: f64 = 0.25;

/// One reading in REST units (temperature °C, humidity %RH, pressure hPa),
/// already rounded like the Python node.
#[derive(Clone, Copy)]
pub struct SensorData {
    pub temperature: f64,
    pub humidity: f64,
    pub pressure: f64,
    pub iaq: i64,
}

pub trait Sensor: Send {
    /// The current reading, or None when the sensor read failed.
    fn read(&mut self) -> Option<SensorData>;
}

use common::{round1, round2, uniform};

/// Rolling-baseline IAQ score ported from the dart_periphery BME680 driver
/// (same algorithm as the Python/ESP32 nodes): a 50-reading gas-resistance
/// window pre-filled with zeros (so the score stabilizes only after ~50
/// readings), gas weighted 75%, humidity 25%.
pub struct IaqState {
    gas_data: [i64; GAS_BURN_IN],
    next: usize,
    last_iaq: i64,
}

impl IaqState {
    pub fn new() -> Self {
        Self {
            gas_data: [0; GAS_BURN_IN],
            next: 0,
            last_iaq: 0,
        }
    }

    pub fn calculate(&mut self, gas_resistance: i64, humidity: f64) -> i64 {
        self.gas_data[self.next] = gas_resistance;
        self.next = (self.next + 1) % GAS_BURN_IN;

        let sum: i64 = self.gas_data.iter().sum();
        let gas_baseline = (sum as f64 / GAS_BURN_IN as f64).round();

        let gas_offset = gas_baseline - gas_resistance as f64;
        let hum_offset = humidity - HUMIDITY_BASELINE;

        let hum_score = if hum_offset > 0.0 {
            (100.0 - HUMIDITY_BASELINE - hum_offset) / (100.0 - HUMIDITY_BASELINE)
                * (HUMIDITY_WEIGHT * 100.0)
        } else {
            (HUMIDITY_BASELINE + hum_offset) / HUMIDITY_BASELINE * (HUMIDITY_WEIGHT * 100.0)
        };

        let gas_weight = 100.0 - HUMIDITY_WEIGHT * 100.0;
        let gas_score = if gas_offset > 0.0 {
            if gas_baseline == 0.0 {
                return self.last_iaq; // matches the Python ZeroDivisionError fallback
            }
            gas_resistance as f64 / gas_baseline * gas_weight
        } else {
            gas_weight
        };

        self.last_iaq = (hum_score + gas_score).round() as i64;
        self.last_iaq
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn current(&self) -> i64 {
        self.last_iaq
    }
}

#[cfg(any(target_os = "linux", test))]
fn iaq_for_sample(
    state: &mut IaqState,
    gas_resistance: f64,
    humidity: f64,
    gas_valid: bool,
    heat_stable: bool,
) -> i64 {
    if gas_valid && heat_stable {
        state.calculate(gas_resistance as i64, humidity)
    } else {
        state.current()
    }
}

// -- Emulated sensor ---------------------------------------------------------

/// Generates plausible BME680 readings without hardware: slow sines for
/// temperature/humidity/pressure and a bounded random walk around 120 kOhm
/// for the gas resistance, scored with the real IAQ algorithm.
pub struct EmulatedBme680 {
    iaq: IaqState,
    gas: f64,
}

impl EmulatedBme680 {
    pub fn new() -> Self {
        Self {
            iaq: IaqState::new(),
            gas: 120_000.0,
        }
    }
}

impl Sensor for EmulatedBme680 {
    fn read(&mut self) -> Option<SensorData> {
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        self.gas = (self.gas + uniform(-2000.0, 2000.0)).clamp(20_000.0, 500_000.0);
        let humidity = 45.0 + 8.0 * (t / 97.0).sin() + uniform(-0.5, 0.5);
        Some(SensorData {
            temperature: round1(22.0 + 2.0 * (t / 60.0).sin() + uniform(-0.1, 0.1)),
            humidity: round1(humidity),
            pressure: round2(1013.0 + 3.0 * (t / 300.0).sin() + uniform(-0.2, 0.2)),
            iaq: self.iaq.calculate(self.gas as i64, humidity),
        })
    }
}

// -- Real sensor (Linux only) ------------------------------------------------

#[cfg(target_os = "linux")]
pub mod real {
    use super::*;
    use crate::driver::{Bme680Driver, DriverReading};

    /// Reads temperature, humidity, pressure and IAQ from a real BME680.
    ///
    pub struct RealBme680 {
        dev: Bme680Driver,
        iaq: IaqState,
    }

    impl RealBme680 {
        pub fn new(i2c_bus: u8) -> Result<Self, String> {
            Ok(Self {
                dev: Bme680Driver::new(i2c_bus)?,
                iaq: IaqState::new(),
            })
        }

        fn measure(&mut self) -> Option<DriverReading> {
            self.dev.read().ok().flatten()
        }
    }

    impl Sensor for RealBme680 {
        fn read(&mut self) -> Option<SensorData> {
            let data = self.measure()?;
            let humidity = data.humidity;
            let iaq = iaq_for_sample(
                &mut self.iaq,
                data.gas_resistance,
                humidity,
                data.gas_valid,
                data.heat_stable,
            );
            Some(SensorData {
                temperature: round1(data.temperature),
                humidity: round1(humidity),
                pressure: round2(data.pressure),
                iaq,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_or_unstable_gas_does_not_advance_baseline() {
        let mut state = IaqState::new();
        assert_eq!(state.current(), 0);
        assert_eq!(iaq_for_sample(&mut state, 0.0, 45.0, false, true), 0);
        assert_eq!(iaq_for_sample(&mut state, 120_000.0, 45.0, true, false), 0);
        assert_eq!(state.next, 0);

        iaq_for_sample(&mut state, 120_000.0, 45.0, true, true);
        assert_eq!(state.next, 1);
    }
}
