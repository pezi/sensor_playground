//! Compact BME680 driver based on the Pimoroni integer implementation, with
//! Bosch-datasheet corrections for signed calibration and gas validity.

#[cfg(target_os = "linux")]
const VARIANT_HIGH: u8 = 0x01;

const LOOKUP_TABLE_1: [i64; 16] = [
    2_147_483_647,
    2_147_483_647,
    2_147_483_647,
    2_147_483_647,
    2_147_483_647,
    2_126_008_810,
    2_147_483_647,
    2_130_303_777,
    2_147_483_647,
    2_147_483_647,
    2_143_188_679,
    2_136_746_228,
    2_147_483_647,
    2_126_008_810,
    2_147_483_647,
    2_147_483_647,
];

const LOOKUP_TABLE_2: [i64; 16] = [
    4_096_000_000,
    2_048_000_000,
    1_024_000_000,
    512_000_000,
    255_744_255,
    127_110_228,
    64_000_000,
    32_258_064,
    16_016_016,
    8_000_000,
    4_000_000,
    2_000_000,
    1_000_000,
    500_000,
    250_000,
    125_000,
];

#[derive(Clone, Copy, Default)]
struct Calibration {
    par_t1: i64,
    par_t2: i64,
    par_t3: i64,
    par_p1: i64,
    par_p2: i64,
    par_p3: i64,
    par_p4: i64,
    par_p5: i64,
    par_p6: i64,
    par_p7: i64,
    par_p8: i64,
    par_p9: i64,
    par_p10: i64,
    par_h1: i64,
    par_h2: i64,
    par_h3: i64,
    par_h4: i64,
    par_h5: i64,
    par_h6: i64,
    par_h7: i64,
    par_gh1: i64,
    par_gh2: i64,
    par_gh3: i64,
    res_heat_range: i64,
    res_heat_val: i64,
    range_sw_err: i64,
}

struct Compensation {
    calibration: Calibration,
    t_fine: i64,
    ambient: i64,
}

impl Compensation {
    fn new(calibration: Calibration) -> Self {
        Self {
            calibration,
            t_fine: 0,
            ambient: 0,
        }
    }

    fn temperature(&mut self, adc: i64) -> i64 {
        let c = self.calibration;
        let var1 = (adc >> 3) - (c.par_t1 << 1);
        let var2 = (var1 * c.par_t2) >> 11;
        let mut var3 = ((var1 >> 1) * (var1 >> 1)) >> 12;
        var3 = (var3 * (c.par_t3 << 4)) >> 14;
        self.t_fine = var2 + var3;
        ((self.t_fine * 5) + 128) >> 8
    }

    fn pressure(&self, adc: i64) -> i64 {
        let c = self.calibration;
        let mut var1 = (self.t_fine >> 1) - 64_000;
        let mut var2 = ((((var1 >> 2) * (var1 >> 2)) >> 11) * c.par_p6) >> 2;
        var2 += (var1 * c.par_p5) << 1;
        var2 = (var2 >> 2) + (c.par_p4 << 16);
        var1 = (((((var1 >> 2) * (var1 >> 2)) >> 13) * (c.par_p3 << 5)) >> 3)
            + ((c.par_p2 * var1) >> 1);
        var1 >>= 18;
        var1 = ((32_768 + var1) * c.par_p1) >> 15;

        let mut press = 1_048_576 - adc;
        press = (press - (var2 >> 12)) * 3_125;
        press = if press >= 1 << 31 {
            floor_div(press, var1) << 1
        } else {
            floor_div(press << 1, var1)
        };

        var1 = (c.par_p9 * (((press >> 3) * (press >> 3)) >> 13)) >> 12;
        var2 = ((press >> 2) * c.par_p8) >> 13;
        let var3 = ((press >> 8) * (press >> 8) * (press >> 8) * c.par_p10) >> 17;
        press + ((var1 + var2 + var3 + (c.par_p7 << 7)) >> 4)
    }

    fn humidity(&self, adc: i64) -> i64 {
        let c = self.calibration;
        let temp_scaled = ((self.t_fine * 5) + 128) >> 8;
        let var1 = (adc - (c.par_h1 * 16)) - (floor_div(temp_scaled * c.par_h3, 100) >> 1);
        let var2 = (c.par_h2
            * (floor_div(temp_scaled * c.par_h4, 100)
                + floor_div(
                    (temp_scaled * floor_div(temp_scaled * c.par_h5, 100)) >> 6,
                    100,
                )
                + 16_384))
            >> 10;
        let var3 = var1 * var2;
        let var4 = ((c.par_h6 << 7) + floor_div(temp_scaled * c.par_h7, 100)) >> 4;
        let var5 = ((var3 >> 14) * (var3 >> 14)) >> 10;
        let var6 = (var4 * var5) >> 1;
        ((((var3 + var6) >> 10) * 1_000) >> 12).clamp(0, 100_000)
    }

    fn gas_resistance_low(&self, adc: i64, gas_range: u8) -> f64 {
        let c = self.calibration;
        let index = gas_range as usize;
        let var1 = ((1_340 + 5 * c.range_sw_err) * LOOKUP_TABLE_1[index]) >> 16;
        let var2 = (adc << 15) - 16_777_216 + var1;
        let var3 = (LOOKUP_TABLE_2[index] * var1) >> 9;
        let mut result = (var3 + (var2 >> 1)) as f64 / var2 as f64;
        if result < 0.0 {
            result += (1_u64 << 32) as f64;
        }
        result
    }

    fn heater_resistance(&self, temperature: i64) -> u8 {
        let temperature = temperature.clamp(200, 400);
        let c = self.calibration;
        let var1 = self.ambient as f64 * c.par_gh3 as f64 / 1_000.0 * 256.0;
        let var2 = (c.par_gh1 + 784) as f64
            * (((c.par_gh2 + 154_009) as f64 * temperature as f64 * 5.0 / 100.0 + 3_276_800.0)
                / 10.0);
        let var3 = var1 + var2 / 2.0;
        let var4 = var3 / (c.res_heat_range + 4) as f64;
        let var5 = 131.0 * c.res_heat_val as f64 + 65_536.0;
        let heater_res_x100 = (var4 / var5 - 250.0) * 34.0;
        ((heater_res_x100 + 50.0) / 100.0) as u8
    }
}

fn floor_div(a: i64, b: i64) -> i64 {
    let quotient = a / b;
    if a % b != 0 && (a < 0) != (b < 0) {
        quotient - 1
    } else {
        quotient
    }
}

fn twos_comp_8(value: u8) -> i64 {
    i64::from(value as i8)
}

fn word(msb: u8, lsb: u8) -> i64 {
    i64::from(u16::from(msb) << 8 | u16::from(lsb))
}

fn word_signed(msb: u8, lsb: u8) -> i64 {
    i64::from((u16::from(msb) << 8 | u16::from(lsb)) as i16)
}

fn parse_calibration(cal: &[u8], heat_range: u8, heat_value: u8, sw_error: u8) -> Calibration {
    Calibration {
        par_t1: word(cal[34], cal[33]),
        par_t2: word_signed(cal[2], cal[1]),
        par_t3: twos_comp_8(cal[3]),
        par_p1: word(cal[6], cal[5]),
        par_p2: word_signed(cal[8], cal[7]),
        par_p3: twos_comp_8(cal[9]),
        par_p4: word_signed(cal[12], cal[11]),
        par_p5: word_signed(cal[14], cal[13]),
        par_p6: twos_comp_8(cal[16]),
        par_p7: twos_comp_8(cal[15]),
        par_p8: word_signed(cal[20], cal[19]),
        par_p9: word_signed(cal[22], cal[21]),
        par_p10: i64::from(cal[23]),
        par_h1: i64::from(cal[27]) << 4 | i64::from(cal[26] & 0x0f),
        par_h2: i64::from(cal[25]) << 4 | i64::from(cal[26] >> 4),
        par_h3: twos_comp_8(cal[28]),
        par_h4: twos_comp_8(cal[29]),
        par_h5: twos_comp_8(cal[30]),
        par_h6: i64::from(cal[31]),
        par_h7: twos_comp_8(cal[32]),
        par_gh1: twos_comp_8(cal[37]),
        par_gh2: word_signed(cal[36], cal[35]),
        par_gh3: twos_comp_8(cal[38]),
        res_heat_range: i64::from((heat_range & 0x30) >> 4),
        res_heat_val: twos_comp_8(heat_value),
        // Register 0x04<7:4> is a signed four-bit field.
        range_sw_err: i64::from(sw_error as i8) >> 4,
    }
}

#[cfg(target_os = "linux")]
fn gas_resistance_high(adc: i64, gas_range: u8) -> f64 {
    let var1 = 262_144_i64 >> gas_range;
    let var2 = (adc - 512) * 3 + 4_096;
    10_000.0 * var1 as f64 / var2 as f64 * 100.0
}

#[cfg(target_os = "linux")]
fn heater_duration(mut duration_ms: f64) -> u8 {
    if duration_ms >= 0xfc0 as f64 {
        return 0xff;
    }
    let mut factor = 0;
    while duration_ms > 0x3f as f64 {
        duration_ms /= 4.0;
        factor += 1;
    }
    (duration_ms + f64::from(factor * 64)) as u8
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use embedded_hal::blocking::i2c::{Write, WriteRead};
    use linux_embedded_hal::I2cdev;
    use std::thread::sleep;
    use std::time::Duration;

    const ADDRESS: u8 = 0x76;
    const CHIP_ID_ADDR: u8 = 0xd0;
    const CHIP_VARIANT_ADDR: u8 = 0xf0;
    const CHIP_ID: u8 = 0x61;
    const SOFT_RESET_ADDR: u8 = 0xe0;
    const SOFT_RESET_CMD: u8 = 0xb6;
    const FIELD0_ADDR: u8 = 0x1d;
    const FIELD_LENGTH: usize = 17;
    const NEW_DATA_MASK: u8 = 0x80;
    const GAS_RANGE_MASK: u8 = 0x0f;
    const HEAT_STABLE_MASK: u8 = 0x10;
    const GAS_VALID_MASK: u8 = 0x20;
    const POLL_PERIOD: Duration = Duration::from_millis(10);

    pub struct DriverReading {
        pub temperature: f64,
        pub pressure: f64,
        pub humidity: f64,
        pub gas_resistance: f64,
        pub gas_valid: bool,
        pub heat_stable: bool,
    }

    pub struct Bme680Driver {
        device: I2cdev,
        compensation: Compensation,
        variant: u8,
    }

    impl Bme680Driver {
        pub fn new(i2c_bus: u8) -> Result<Self, String> {
            let device = I2cdev::new(format!("/dev/i2c-{i2c_bus}"))
                .map_err(|error| format!("opening /dev/i2c-{i2c_bus}: {error}"))?;
            let mut sensor = Self {
                device,
                compensation: Compensation::new(Calibration::default()),
                variant: 0,
            };

            let id = sensor.read_register(CHIP_ID_ADDR)?;
            if id != CHIP_ID {
                return Err(format!("BME680 not found, invalid chip id 0x{id:02x}"));
            }
            sensor.variant = sensor.read_register(CHIP_VARIANT_ADDR)?;
            sensor.write_register(SOFT_RESET_ADDR, SOFT_RESET_CMD)?;
            sleep(Duration::from_millis(10));

            sensor.read_calibration()?;

            // Humidity 2x, pressure 4x, temperature 8x, IIR filter 3.
            sensor.set_bits(0x72, 0x07, 0, 2)?;
            sensor.set_bits(0x74, 0x1c, 2, 3)?;
            sensor.set_bits(0x74, 0xe0, 5, 4)?;
            sensor.set_bits(0x75, 0x1c, 2, 2)?;
            let run_gas = if sensor.variant == VARIANT_HIGH { 2 } else { 1 };
            sensor.set_bits(0x71, 0x30, 4, run_gas)?;

            if sensor.read()?.is_none() {
                return Err("initial BME680 measurement timed out".into());
            }
            sensor.write_register(0x5a, sensor.compensation.heater_resistance(320))?;
            sensor.write_register(0x64, heater_duration(150.0))?;
            Ok(sensor)
        }

        pub fn read(&mut self) -> Result<Option<DriverReading>, String> {
            // The status bit can still describe the preceding forced sample.
            // Require the measurement index to advance after forcing a sample.
            let previous_index = self.read_register(FIELD0_ADDR + 1)?;
            self.set_bits(0x74, 0x03, 0, 1)?;

            for _ in 0..30 {
                if self.read_register(FIELD0_ADDR)? & NEW_DATA_MASK == 0 {
                    sleep(POLL_PERIOD);
                    continue;
                }

                let registers = self.read_registers(FIELD0_ADDR, FIELD_LENGTH)?;
                if registers[1] == previous_index {
                    sleep(POLL_PERIOD);
                    continue;
                }

                let adc_pressure = i64::from(registers[2]) << 12
                    | i64::from(registers[3]) << 4
                    | i64::from(registers[4]) >> 4;
                let adc_temperature = i64::from(registers[5]) << 12
                    | i64::from(registers[6]) << 4
                    | i64::from(registers[7]) >> 4;
                let adc_humidity = i64::from(registers[8]) << 8 | i64::from(registers[9]);
                let adc_gas_low = i64::from(registers[13]) << 2 | i64::from(registers[14]) >> 6;
                let adc_gas_high = i64::from(registers[15]) << 2 | i64::from(registers[16]) >> 6;
                let gas_range_low = registers[14] & GAS_RANGE_MASK;
                let gas_range_high = registers[16] & GAS_RANGE_MASK;
                let gas_status = if self.variant == VARIANT_HIGH {
                    registers[16]
                } else {
                    registers[14]
                };

                let temperature_x100 = self.compensation.temperature(adc_temperature);
                self.compensation.ambient = temperature_x100;
                let gas_resistance = if self.variant == VARIANT_HIGH {
                    gas_resistance_high(adc_gas_high, gas_range_high)
                } else {
                    self.compensation
                        .gas_resistance_low(adc_gas_low, gas_range_low)
                };

                return Ok(Some(DriverReading {
                    temperature: temperature_x100 as f64 / 100.0,
                    pressure: self.compensation.pressure(adc_pressure) as f64 / 100.0,
                    humidity: self.compensation.humidity(adc_humidity) as f64 / 1_000.0,
                    gas_resistance,
                    gas_valid: gas_status & GAS_VALID_MASK != 0,
                    heat_stable: gas_status & HEAT_STABLE_MASK != 0,
                }));
            }
            Ok(None)
        }

        fn read_calibration(&mut self) -> Result<(), String> {
            let mut raw = self.read_registers(0x89, 25)?;
            raw.extend(self.read_registers(0xe1, 16)?);
            let heat_range = self.read_register(0x02)?;
            let heat_value = self.read_register(0x00)?;
            let switch_error = self.read_register(0x04)?;
            self.compensation = Compensation::new(parse_calibration(
                &raw,
                heat_range,
                heat_value,
                switch_error,
            ));
            Ok(())
        }

        fn read_register(&mut self, register: u8) -> Result<u8, String> {
            Ok(self.read_registers(register, 1)?[0])
        }

        fn read_registers(&mut self, register: u8, length: usize) -> Result<Vec<u8>, String> {
            let mut buffer = vec![0; length];
            self.device
                .write_read(ADDRESS, &[register], &mut buffer)
                .map_err(|error| format!("BME680 I2C read at 0x{register:02x}: {error:?}"))?;
            Ok(buffer)
        }

        fn write_register(&mut self, register: u8, value: u8) -> Result<(), String> {
            self.device
                .write(ADDRESS, &[register, value])
                .map_err(|error| format!("BME680 I2C write at 0x{register:02x}: {error:?}"))
        }

        fn set_bits(
            &mut self,
            register: u8,
            mask: u8,
            position: u8,
            value: u8,
        ) -> Result<(), String> {
            let current = self.read_register(register)?;
            self.write_register(register, (current & !mask) | (value << position))
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::{Bme680Driver, DriverReading};

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Deserialize)]
    struct GoldenCase {
        adc_t: i64,
        adc_p: i64,
        adc_h: i64,
        adc_g: i64,
        range: u8,
        t: i64,
        tfine: i64,
        p: i64,
        h: i64,
        g: f64,
        heat: u8,
    }

    #[derive(Deserialize)]
    struct Goldens {
        cal: Vec<u8>,
        heat_range_reg: u8,
        heat_val: u8,
        sw_err_reg: u8,
        cases: Vec<GoldenCase>,
    }

    #[test]
    fn compensation_matches_corrected_goldens() {
        let goldens: Goldens = serde_json::from_str(include_str!(
            "../../../go/bme680/testdata/compensation_goldens.json"
        ))
        .unwrap();
        let calibration = parse_calibration(
            &goldens.cal,
            goldens.heat_range_reg,
            goldens.heat_val,
            goldens.sw_err_reg,
        );
        assert_eq!(calibration.range_sw_err, -4);
        let mut compensation = Compensation::new(calibration);

        for (index, golden) in goldens.cases.iter().enumerate() {
            let temperature = compensation.temperature(golden.adc_t);
            assert_eq!(temperature, golden.t, "case {index}: temperature");
            assert_eq!(compensation.t_fine, golden.tfine, "case {index}: t_fine");
            assert_eq!(
                compensation.pressure(golden.adc_p),
                golden.p,
                "case {index}: pressure"
            );
            assert_eq!(
                compensation.humidity(golden.adc_h),
                golden.h,
                "case {index}: humidity"
            );
            let gas = compensation.gas_resistance_low(golden.adc_g, golden.range);
            assert!(
                (gas - golden.g).abs() < 1e-6,
                "case {index}: gas {gas} != {}",
                golden.g
            );
            compensation.ambient = temperature;
            assert_eq!(
                compensation.heater_resistance(320),
                golden.heat,
                "case {index}: heater"
            );
        }
    }

    #[test]
    fn gas_lookup_range_twelve_is_not_zero() {
        let compensation = Compensation::new(Calibration::default());
        assert!(compensation.gas_resistance_low(700, 12) > 0.0);
    }
}
