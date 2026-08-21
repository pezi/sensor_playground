'use strict';
/*
 * Compact VL53L0X driver, ported from the Adafruit CircuitPython driver
 * adafruit_vl53l0x (https://github.com/adafruit/Adafruit_CircuitPython_VL53L0X,
 * MIT) — the library the Python node uses, itself adapted from the Pololu
 * vl53l0x-arduino code: the full init register sequence (reference SPAD
 * selection, tuning settings, interrupt config, timing-budget preservation,
 * VHV/phase ref calibration), then single-shot ranging reads in millimeters.
 * The timeout encoding and timing-budget math are platform-neutral (and
 * unit tested); only the I2C access is Linux-only.
 *
 * Like the Python node (which leaves the driver's io_timeout_s at 0), the
 * wait loops poll without a timeout — an unresponsive sensor surfaces as an
 * I2C error, not a hang, because every poll is a bus transaction.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const VL53L0X_ADDR = 0x29; // the fixed power-on address (41)

// Registers (the subset the driver uses, names from the Adafruit driver).
const SYSRANGE_START = 0x00;
const SYSTEM_SEQUENCE_CONFIG = 0x01;
const SYSTEM_INTERRUPT_CONFIG_GPIO = 0x0a;
const SYSTEM_INTERRUPT_CLEAR = 0x0b;
const RESULT_INTERRUPT_STATUS = 0x13;
const RESULT_RANGE_STATUS = 0x14;
const FINAL_RANGE_CONFIG_MIN_COUNT_RATE_RTN_LIMIT = 0x44;
const MSRC_CONFIG_TIMEOUT_MACROP = 0x46;
const DYNAMIC_SPAD_NUM_REQUESTED_REF_SPAD = 0x4e;
const DYNAMIC_SPAD_REF_EN_START_OFFSET = 0x4f;
const PRE_RANGE_CONFIG_VCSEL_PERIOD = 0x50;
const PRE_RANGE_CONFIG_TIMEOUT_MACROP_HI = 0x51;
const MSRC_CONFIG_CONTROL = 0x60;
const FINAL_RANGE_CONFIG_VCSEL_PERIOD = 0x70;
const FINAL_RANGE_CONFIG_TIMEOUT_MACROP_HI = 0x71;
const GPIO_HV_MUX_ACTIVE_HIGH = 0x84;
const GLOBAL_CONFIG_SPAD_ENABLES_REF_0 = 0xb0;
const GLOBAL_CONFIG_REF_EN_START_SELECT = 0xb6;

// -- Platform-neutral math (mirrors the Adafruit driver's helpers) -----------

/** Decode a timeout register value; format: "(LSByte * 2^MSByte) + 1". */
function decodeTimeout(val) {
  return (val & 0xff) * Math.pow(2.0, (val & 0xff00) >> 8) + 1;
}

/** Encode a timeout in MCLKs into the register format "(LSByte * 2^MSByte) + 1". */
function encodeTimeout(timeoutMclks) {
  let mclks = Math.trunc(timeoutMclks) & 0xffff;
  let lsByte = 0;
  let msByte = 0;
  if (mclks > 0) {
    lsByte = mclks - 1;
    while (lsByte > 255) {
      lsByte >>= 1;
      msByte += 1;
    }
    return ((msByte << 8) | (lsByte & 0xff)) & 0xffff;
  }
  return 0;
}

/*
 * Timeout MCLK <-> microsecond conversion for the given VCSEL period
 * (integer-floor arithmetic like the reference driver).
 */
function timeoutMclksToUs(timeoutPeriodMclks, vcselPeriodPclks) {
  const macroPeriodNs = Math.floor((2304 * vcselPeriodPclks * 1655 + 500) / 1000);
  return Math.floor((timeoutPeriodMclks * macroPeriodNs + Math.floor(macroPeriodNs / 2)) / 1000);
}

function timeoutUsToMclks(timeoutPeriodUs, vcselPeriodPclks) {
  const macroPeriodNs = Math.floor((2304 * vcselPeriodPclks * 1655 + 500) / 1000);
  return Math.floor((timeoutPeriodUs * 1000 + Math.floor(macroPeriodNs / 2)) / macroPeriodNs);
}

/** Decode a VCSEL period register value into PCLKs. */
function decodeVcselPeriod(regVal) {
  return ((regVal + 1) & 0xff) << 1;
}

/*
 * Decode SYSTEM_SEQUENCE_CONFIG (based on VL53L0X_GetSequenceStepEnables
 * from the ST API).
 */
function sequenceStepEnables(sequenceConfig) {
  return {
    tcc: ((sequenceConfig >> 4) & 0x1) > 0,
    dss: ((sequenceConfig >> 3) & 0x1) > 0,
    msrc: ((sequenceConfig >> 2) & 0x1) > 0,
    preRange: ((sequenceConfig >> 6) & 0x1) > 0,
    finalRange: ((sequenceConfig >> 7) & 0x1) > 0,
  };
}

/*
 * Clear every SPAD enable bit outside the good reference window (the
 * first 12 bits are aperture SPADs) and past spadCount enabled ones,
 * mutating the 6-byte map; returns how many stayed enabled.
 */
function maskRefSpadMap(refSpadMap, spadCount, spadIsAperture) {
  const firstSpadToEnable = spadIsAperture ? 12 : 0;
  let spadsEnabled = 0;
  for (let i = 0; i < 48; i++) {
    if (i < firstSpadToEnable || spadsEnabled === spadCount) {
      // This bit is lower than the first one that should be enabled, or
      // (reference_spad_count) bits have already been enabled, so zero
      // this bit.
      refSpadMap[Math.floor(i / 8)] &= ~(1 << i % 8);
    } else if ((refSpadMap[Math.floor(i / 8)] >> i % 8) & 0x1) {
      spadsEnabled += 1;
    }
  }
  return spadsEnabled;
}

/*
 * The block of undocumented "default tuning settings" every VL53L0X
 * driver writes after the SPAD map (ST API load_tuning_settings).
 */
const TUNING = [
  [0xff, 0x01], [0x00, 0x00], [0xff, 0x00], [0x09, 0x00], [0x10, 0x00],
  [0x11, 0x00], [0x24, 0x01], [0x25, 0xff], [0x75, 0x00], [0xff, 0x01],
  [0x4e, 0x2c], [0x48, 0x00], [0x30, 0x20], [0xff, 0x00], [0x30, 0x09],
  [0x54, 0x00], [0x31, 0x04], [0x32, 0x03], [0x40, 0x83], [0x46, 0x25],
  [0x60, 0x00], [0x27, 0x00], [0x50, 0x06], [0x51, 0x00], [0x52, 0x96],
  [0x56, 0x08], [0x57, 0x30], [0x61, 0x00], [0x62, 0x00], [0x64, 0x00],
  [0x65, 0x00], [0x66, 0xa0], [0xff, 0x01], [0x22, 0x32], [0x47, 0x14],
  [0x49, 0xff], [0x4a, 0x00], [0xff, 0x00], [0x7a, 0x0a], [0x7b, 0x00],
  [0x78, 0x21], [0xff, 0x01], [0x23, 0x34], [0x42, 0x00], [0x44, 0xff],
  [0x45, 0x26], [0x46, 0x05], [0x40, 0x40], [0x0e, 0x06], [0x20, 0x1a],
  [0x43, 0x40], [0xff, 0x00], [0x34, 0x03], [0x35, 0x44], [0xff, 0x01],
  [0x31, 0x04], [0x4b, 0x09], [0x4c, 0x05], [0x4d, 0x04], [0xff, 0x00],
  [0x44, 0x00], [0x45, 0x20], [0x47, 0x08], [0x48, 0x28], [0x67, 0x00],
  [0x70, 0x04], [0x71, 0x01], [0x72, 0xfe], [0x76, 0x00], [0x77, 0x00],
  [0xff, 0x01], [0x0d, 0x01], [0xff, 0x00], [0x80, 0x01], [0x01, 0xf8],
  [0xff, 0x01], [0x8e, 0x01], [0x00, 0x01], [0xff, 0x00], [0x80, 0x00],
];

// -- Device ------------------------------------------------------------------

class VL53L0XDriver {
  /** Open the sensor on /dev/i2c-<bus> at 0x29 and run the full init sequence. */
  static async create(busNumber) {
    let i2c;
    try {
      i2c = require('i2c-bus');
    } catch {
      console.log(
        "Error: the i2c-bus package is not installed. It is an *optional* npm\n" +
        "dependency (so a failed native build does not stop `npm install`).\n" +
        "In this node's folder run:\n" +
        "    sudo apt install -y build-essential python3\n" +
        "    npm install\n" +
        "and check the output for i2c-bus build errors. Or set\n" +
        '"emulation": true in config.json to run without hardware.'
      );
      process.exit(1);
    }
    const bus = await i2c.openPromisified(busNumber);
    const driver = new VL53L0XDriver(bus);
    await driver._init();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
    this._addr = VL53L0X_ADDR;
    this._stopVariable = 0;
  }

  async _readU8(reg) {
    return this._bus.readByte(this._addr, reg);
  }

  async _readU16(reg) {
    const buf = Buffer.alloc(2);
    await this._bus.readI2cBlock(this._addr, reg, 2, buf);
    return (buf[0] << 8) | buf[1]; // big-endian
  }

  async _writeU8(reg, val) {
    await this._bus.writeByte(this._addr, reg, val & 0xff);
  }

  async _writeU16(reg, val) {
    await this._bus.writeI2cBlock(this._addr, reg, 2,
      Buffer.from([(val >> 8) & 0xff, val & 0xff])); // big-endian
  }

  async _writePairs(pairs) {
    for (const [reg, val] of pairs) await this._writeU8(reg, val);
  }

  /*
   * Mirrors the Adafruit driver's __init__ (data init, static init, SPAD
   * management, tuning settings, interrupt config, timing budget, ref
   * calibration).
   */
  async _init() {
    // Check identification registers for expected values (datasheet 3.2).
    if (
      (await this._readU8(0xc0)) !== 0xee ||
      (await this._readU8(0xc1)) !== 0xaa ||
      (await this._readU8(0xc2)) !== 0x10
    ) {
      throw new Error('Failed to find expected ID register values. Check wiring!');
    }
    // Set I2C standard mode.
    await this._writePairs([[0x88, 0x00], [0x80, 0x01], [0xff, 0x01], [0x00, 0x00]]);
    this._stopVariable = await this._readU8(0x91);
    await this._writePairs([[0x00, 0x01], [0xff, 0x00], [0x80, 0x00]]);
    // Disable SIGNAL_RATE_MSRC (bit 1) and SIGNAL_RATE_PRE_RANGE (bit 4)
    // limit checks.
    const configControl = (await this._readU8(MSRC_CONFIG_CONTROL)) | 0x12;
    await this._writeU8(MSRC_CONFIG_CONTROL, configControl);
    // Set final range signal rate limit to 0.25 MCPS (million counts per
    // second), as 16-bit 9.7 fixed point.
    await this._writeU16(FINAL_RANGE_CONFIG_MIN_COUNT_RATE_RTN_LIMIT, Math.trunc(0.25 * (1 << 7)));
    await this._writeU8(SYSTEM_SEQUENCE_CONFIG, 0xff);

    const [spadCount, spadIsAperture] = await this._getSpadInfo();
    // The SPAD map (RefGoodSpadMap) is read by VL53L0X_get_info_from_device()
    // in the API, but the same data is more easily readable from
    // GLOBAL_CONFIG_SPAD_ENABLES_REF_0 through _6, so read it from there.
    const refSpadMap = Buffer.alloc(6);
    await this._bus.readI2cBlock(this._addr, GLOBAL_CONFIG_SPAD_ENABLES_REF_0, 6, refSpadMap);
    await this._writePairs([
      [0xff, 0x01],
      [DYNAMIC_SPAD_REF_EN_START_OFFSET, 0x00],
      [DYNAMIC_SPAD_NUM_REQUESTED_REF_SPAD, 0x2c],
      [0xff, 0x00],
      [GLOBAL_CONFIG_REF_EN_START_SELECT, 0xb4],
    ]);
    maskRefSpadMap(refSpadMap, spadCount, spadIsAperture);
    await this._bus.writeI2cBlock(this._addr, GLOBAL_CONFIG_SPAD_ENABLES_REF_0, 6, refSpadMap);

    await this._writePairs(TUNING);
    await this._writeU8(SYSTEM_INTERRUPT_CONFIG_GPIO, 0x04);
    const gpioHvMux = await this._readU8(GPIO_HV_MUX_ACTIVE_HIGH);
    await this._writeU8(GPIO_HV_MUX_ACTIVE_HIGH, gpioHvMux & ~0x10); // active low
    await this._writeU8(SYSTEM_INTERRUPT_CLEAR, 0x01);

    const budgetUs = await this._getMeasurementTimingBudget();
    await this._writeU8(SYSTEM_SEQUENCE_CONFIG, 0xe8);
    await this._setMeasurementTimingBudget(budgetUs);
    await this._writeU8(SYSTEM_SEQUENCE_CONFIG, 0x01);
    await this._performSingleRefCalibration(0x40);
    await this._writeU8(SYSTEM_SEQUENCE_CONFIG, 0x02);
    await this._performSingleRefCalibration(0x00);
    // "restore the previous Sequence Config"
    await this._writeU8(SYSTEM_SEQUENCE_CONFIG, 0xe8);
  }

  /*
   * Reference SPAD count and type (is_aperture), based on the Pololu
   * vl53l0x-arduino code.
   */
  async _getSpadInfo() {
    await this._writePairs([[0x80, 0x01], [0xff, 0x01], [0x00, 0x00], [0xff, 0x06]]);
    await this._writeU8(0x83, (await this._readU8(0x83)) | 0x04);
    await this._writePairs([
      [0xff, 0x07], [0x81, 0x01], [0x80, 0x01], [0x94, 0x6b], [0x83, 0x00],
    ]);
    while ((await this._readU8(0x83)) === 0x00) {
      // wait for the device (no timeout, like the Python node)
    }
    await this._writeU8(0x83, 0x01);
    const tmp = await this._readU8(0x92);
    const count = tmp & 0x7f;
    const isAperture = ((tmp >> 7) & 0x01) === 1;
    await this._writePairs([[0x81, 0x00], [0xff, 0x06]]);
    await this._writeU8(0x83, (await this._readU8(0x83)) & ~0x04);
    await this._writePairs([[0xff, 0x01], [0x00, 0x01], [0xff, 0x00], [0x80, 0x00]]);
    return [count, isAperture];
  }

  /** Based on VL53L0X_perform_single_ref_calibration() from the ST API. */
  async _performSingleRefCalibration(vhvInitByte) {
    await this._writeU8(SYSRANGE_START, 0x01 | (vhvInitByte & 0xff));
    while (((await this._readU8(RESULT_INTERRUPT_STATUS)) & 0x07) === 0) {
      // wait for the device (no timeout, like the Python node)
    }
    await this._writeU8(SYSTEM_INTERRUPT_CLEAR, 0x01);
    await this._writeU8(SYSRANGE_START, 0x00);
  }

  async _getVcselPulsePeriod(reg) {
    return decodeVcselPeriod(await this._readU8(reg));
  }

  /*
   * Based on get_sequence_step_timeout() from the ST API, modified like
   * the Pololu code.
   */
  async _getSequenceStepTimeouts(preRange) {
    const preRangeVcselPeriodPclks = await this._getVcselPulsePeriod(PRE_RANGE_CONFIG_VCSEL_PERIOD);
    const msrcDssTccMclks = ((await this._readU8(MSRC_CONFIG_TIMEOUT_MACROP)) + 1) & 0xff;
    const msrcDssTccUs = timeoutMclksToUs(msrcDssTccMclks, preRangeVcselPeriodPclks);
    const preRangeMclks = decodeTimeout(await this._readU16(PRE_RANGE_CONFIG_TIMEOUT_MACROP_HI));
    const preRangeUs = timeoutMclksToUs(preRangeMclks, preRangeVcselPeriodPclks);
    const finalRangeVcselPeriodPclks =
      await this._getVcselPulsePeriod(FINAL_RANGE_CONFIG_VCSEL_PERIOD);
    let finalRangeMclks = decodeTimeout(await this._readU16(FINAL_RANGE_CONFIG_TIMEOUT_MACROP_HI));
    if (preRange) finalRangeMclks -= preRangeMclks;
    const finalRangeUs = timeoutMclksToUs(finalRangeMclks, finalRangeVcselPeriodPclks);
    return { msrcDssTccUs, preRangeUs, finalRangeUs, finalRangeVcselPeriodPclks, preRangeMclks };
  }

  /** The measurement timing budget in microseconds. */
  async _getMeasurementTimingBudget() {
    let budgetUs = 1910 + 960; // start overhead + end overhead
    const enables = sequenceStepEnables(await this._readU8(SYSTEM_SEQUENCE_CONFIG));
    const t = await this._getSequenceStepTimeouts(enables.preRange);
    if (enables.tcc) budgetUs += t.msrcDssTccUs + 590;
    if (enables.dss) budgetUs += 2 * (t.msrcDssTccUs + 690);
    else if (enables.msrc) budgetUs += t.msrcDssTccUs + 660;
    if (enables.preRange) budgetUs += t.preRangeUs + 660;
    if (enables.finalRange) budgetUs += t.finalRangeUs + 550;
    return budgetUs;
  }

  /*
   * Apply a measurement timing budget in microseconds by giving the final
   * range step whatever time the other enabled steps leave over.
   */
  async _setMeasurementTimingBudget(budgetUs) {
    if (budgetUs < 20000) {
      throw new Error(`Timing budget ${budgetUs} us is below the 20000 us minimum`);
    }
    let usedBudgetUs = 1320 + 960; // start (diff from get) + end overhead
    const enables = sequenceStepEnables(await this._readU8(SYSTEM_SEQUENCE_CONFIG));
    const t = await this._getSequenceStepTimeouts(enables.preRange);
    if (enables.tcc) usedBudgetUs += t.msrcDssTccUs + 590;
    if (enables.dss) usedBudgetUs += 2 * (t.msrcDssTccUs + 690);
    else if (enables.msrc) usedBudgetUs += t.msrcDssTccUs + 660;
    if (enables.preRange) usedBudgetUs += t.preRangeUs + 660;
    if (enables.finalRange) {
      usedBudgetUs += 550;
      // "Note that the final range timeout is determined by the timing
      // budget and the sum of all other timeouts within the sequence.
      // If there is no room for the final range timeout, then an error
      // will be set. Otherwise the remaining time will be applied to
      // the final range."
      if (usedBudgetUs > budgetUs) throw new Error('Requested timeout too big.');
      let finalRangeTimeoutMclks =
        timeoutUsToMclks(budgetUs - usedBudgetUs, t.finalRangeVcselPeriodPclks);
      if (enables.preRange) finalRangeTimeoutMclks += t.preRangeMclks;
      await this._writeU16(FINAL_RANGE_CONFIG_TIMEOUT_MACROP_HI,
        encodeTimeout(finalRangeTimeoutMclks));
    }
  }

  /*
   * One single-shot range measurement in millimeters (adapted from
   * readRangeSingleMillimeters in the Pololu code; assumes the default
   * linearity corrective gain of 1000 and no fractional ranging).
   */
  async readRange() {
    await this._writePairs([
      [0x80, 0x01],
      [0xff, 0x01],
      [0x00, 0x00],
      [0x91, this._stopVariable],
      [0x00, 0x01],
      [0xff, 0x00],
      [0x80, 0x00],
      [SYSRANGE_START, 0x01],
    ]);
    while (((await this._readU8(SYSRANGE_START)) & 0x01) > 0) {
      // wait for the device (no timeout, like the Python node)
    }
    while (((await this._readU8(RESULT_INTERRUPT_STATUS)) & 0x07) === 0) {
      // wait for the device (no timeout, like the Python node)
    }
    const rangeMm = await this._readU16(RESULT_RANGE_STATUS + 10);
    await this._writeU8(SYSTEM_INTERRUPT_CLEAR, 0x01);
    return rangeMm;
  }
}

module.exports = {
  VL53L0XDriver,
  decodeTimeout,
  encodeTimeout,
  timeoutMclksToUs,
  timeoutUsToMclks,
  decodeVcselPeriod,
  sequenceStepEnables,
  maskRefSpadMap,
};
