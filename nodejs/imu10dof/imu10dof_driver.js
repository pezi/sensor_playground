'use strict';
/*
 * Drivers for the Grove IMU 10DOF board: the MPU9250 accelerometer +
 * AK8963 magnetometer, ported from the mpu9250-jmdev Python driver (the
 * library the Python node uses — bypass mode, 8 g / 16-bit full scale,
 * 100 Hz continuous magnetometer), and the BMP280 barometer with the
 * Bosch datasheet integer compensation, ported from the Python node's
 * own BMP280 class — so the readings match the Python node. The 64-bit
 * pressure algorithm needs BigInt.
 *
 * Requires the `i2c-bus` package (Linux only) — loaded lazily so that
 * emulation mode works without it.
 */

const MPU_ADDRESS = 0x68; // MPU9250 (MPU9050_ADDRESS_68)
const AK_ADDRESS = 0x0c; // AK8963 magnetometer, visible in bypass mode
const BMP_ADDRESS = 0x77; // BMP280 barometer

// MPU9250 registers.
const MPU_SMPLRT_DIV = 0x19;
const MPU_CONFIG = 0x1a;
const MPU_GYRO_CONFIG = 0x1b;
const MPU_ACCEL_CONFIG = 0x1c;
const MPU_ACCEL_CONFIG_2 = 0x1d;
const MPU_INT_PIN_CFG = 0x37;
const MPU_ACCEL_OUT = 0x3b;
const MPU_USER_CTRL = 0x6a;
const MPU_PWR_MGMT_1 = 0x6b;

// AK8963 registers.
const AK_MAGNET_OUT = 0x03; // HXL..HZH + ST2 (7 bytes)
const AK_CNTL1 = 0x0a;
const AK_ASAX = 0x10; // factory sensitivity (FuseROM, 3 bytes)

// Full-scale selections the Python node configures.
const GFS_1000 = 0x02; // gyro 1000 dps
const AFS_8G = 0x02; // accel 8 g
const AK_BIT_16 = 0x01; // magnetometer 16-bit output
const AK_MODE_C100HZ = 0x06; // continuous 100 Hz

const ACCEL_SCALE = 8.0 / 32768.0; // ACCEL_SCALE_MODIFIER_8G
const MAG_SCALE = 4912.0 / 32760.0; // MAGNOMETER_SCALE_MODIFIER_BIT_16

const BMP_CHIP_ID = 0x58; // value of the id register (0xD0) for the BMP280

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/*
 * Scale a big-endian 6-byte accelerometer block to g at the 8 g full
 * scale.
 */
function convertAccel(data) {
  const s16 = (msb, lsb) => {
    const w = (msb << 8) | lsb;
    return w > 32767 ? w - 65536 : w;
  };
  return {
    x: s16(data[0], data[1]) * ACCEL_SCALE,
    y: s16(data[2], data[3]) * ACCEL_SCALE,
    z: s16(data[4], data[5]) * ACCEL_SCALE,
  };
}

/*
 * Scale a little-endian 7-byte magnetometer block (HXL..ST2) to µT at
 * the 16-bit full scale, applying the factory sensitivity. A set ST2
 * overflow bit yields zeros, like the reference driver.
 */
function convertMag(data, magCal) {
  if ((data[6] & 0x08) === 0x08) {
    return { x: 0, y: 0, z: 0 }; // magnetic sensor overflow
  }
  const s16 = (lsb, msb) => {
    const w = (msb << 8) | lsb;
    return w > 32767 ? w - 65536 : w;
  };
  return {
    x: s16(data[0], data[1]) * MAG_SCALE * magCal[0],
    y: s16(data[2], data[3]) * MAG_SCALE * magCal[1],
    z: s16(data[4], data[5]) * MAG_SCALE * magCal[2],
  };
}

/*
 * Reduce the MPU9250 axes to the node's derived readings: roll/pitch
 * from the accelerometer, compass heading from the raw magnetometer
 * (not tilt-compensated) and the total acceleration magnitude (g-force).
 */
function computeAngles(ax, ay, az, mx, my) {
  const degrees = (rad) => (rad * 180.0) / Math.PI;
  let heading = degrees(Math.atan2(my, mx)) % 360.0;
  if (heading < 0) heading += 360.0;
  return {
    roll: degrees(Math.atan2(ay, az)),
    pitch: degrees(Math.atan2(-ax, Math.sqrt(ay * ay + az * az))),
    heading,
    gforce: Math.sqrt(ax * ax + ay * ay + az * az),
  };
}

/*
 * Parse the little-endian dig_T*\/dig_P* calibration words (T1/P1
 * unsigned, the rest signed).
 */
function parseBmp280Calibration(calib) {
  const u16 = (i) => calib[i] | (calib[i + 1] << 8);
  const s16 = (i) => {
    const w = u16(i);
    return w > 32767 ? w - 65536 : w;
  };
  return {
    t1: u16(0), t2: s16(2), t3: s16(4),
    p1: u16(6), p2: s16(8), p3: s16(10),
    p4: s16(12), p5: s16(14), p6: s16(16),
    p7: s16(18), p8: s16(20), p9: s16(22),
  };
}

/*
 * Python's // operator: the compensation's division rounds toward
 * negative infinity, BigInt division toward zero.
 */
function floorDiv(a, b) {
  const q = a / b;
  return a % b !== 0n && a < 0n !== b < 0n ? q - 1n : q;
}

/*
 * Raw ADC values -> { temperature °C, pressure hPa } with the Bosch
 * datasheet integer algorithms (32-bit temperature, 64-bit pressure),
 * exactly as transcribed in the Python node.
 */
function compensateBmp280(cal, adcT, adcP) {
  const t1 = BigInt(cal.t1); const t2 = BigInt(cal.t2); const t3 = BigInt(cal.t3);
  const p1 = BigInt(cal.p1); const p2 = BigInt(cal.p2); const p3 = BigInt(cal.p3);
  const p4 = BigInt(cal.p4); const p5 = BigInt(cal.p5); const p6 = BigInt(cal.p6);
  const p7 = BigInt(cal.p7); const p8 = BigInt(cal.p8); const p9 = BigInt(cal.p9);
  const t = BigInt(adcT);

  // Temperature compensation (datasheet 32-bit integer algorithm).
  let var1 = (((t >> 3n) - (t1 << 1n)) * t2) >> 11n;
  let var2 = (((((t >> 4n) - t1) * ((t >> 4n) - t1)) >> 12n) * t3) >> 14n;
  const tFine = var1 + var2;
  const temperature = Number((tFine * 5n + 128n) >> 8n) / 100.0;

  // Pressure compensation (datasheet 64-bit integer algorithm).
  var1 = tFine - 128000n;
  var2 = var1 * var1 * p6;
  var2 += (var1 * p5) << 17n;
  var2 += p4 << 35n;
  var1 = ((var1 * var1 * p3) >> 8n) + ((var1 * p2) << 12n);
  var1 = (((1n << 47n) + var1) * p1) >> 33n;
  if (var1 === 0n) {
    return { temperature, pressure: 0.0 }; // avoid division by zero
  }
  let p = 1048576n - BigInt(adcP);
  p = floorDiv(((p << 31n) - var2) * 3125n, var1);
  var1 = (p9 * (p >> 13n) * (p >> 13n)) >> 25n;
  var2 = (p8 * p) >> 19n;
  p = ((p + var1 + var2) >> 8n) + (p7 << 4n);

  // p is in Q24.8 Pa; convert to hPa.
  return { temperature, pressure: Number(p) / 256.0 / 100.0 };
}

/** Reads roll/pitch/heading/g-force plus temperature and pressure. */
class IMU10DOFDriver {
  /*
   * Open the MPU9250, its AK8963 and the BMP280 on /dev/i2c-<bus> and
   * run the configuration sequences.
   */
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
    const driver = new IMU10DOFDriver(bus);
    await driver._configureMpu();
    await driver._configureBmp();
    return driver;
  }

  constructor(bus) {
    this._bus = bus;
    this._magCal = [1, 1, 1]; // AK8963 factory sensitivity adjustment
    this._bmpCal = null;
  }

  async _write(addr, reg, val, waitMs = 0) {
    await this._bus.writeByte(addr, reg, val);
    if (waitMs > 0) await sleep(waitMs);
  }

  async _readBlock(addr, reg, length) {
    const buf = Buffer.alloc(length);
    await this._bus.readI2cBlock(addr, reg, length, buf);
    return buf;
  }

  /*
   * The mpu9250-jmdev configuration sequence (retried up to three
   * times, like the reference driver).
   */
  async _configureMpu(retry = 3) {
    try {
      // MPU6500 core (configureMPU6500, no slave).
      await this._write(MPU_ADDRESS, MPU_PWR_MGMT_1, 0x00, 100); // sleep off
      await this._write(MPU_ADDRESS, MPU_PWR_MGMT_1, 0x01, 100); // auto select clock source
      await this._write(MPU_ADDRESS, MPU_CONFIG, 0x00); // DLPF_CFG
      await this._write(MPU_ADDRESS, MPU_SMPLRT_DIV, 0x00); // sample rate divider
      await this._write(MPU_ADDRESS, MPU_GYRO_CONFIG, GFS_1000 << 3); // gyro full scale select
      await this._write(MPU_ADDRESS, MPU_ACCEL_CONFIG, AFS_8G << 3); // accel full scale select
      await this._write(MPU_ADDRESS, MPU_ACCEL_CONFIG_2, 0x00); // A_DLPFCFG
      await this._write(MPU_ADDRESS, MPU_INT_PIN_CFG, 0x02, 100); // BYPASS_EN enable
      await this._write(MPU_ADDRESS, MPU_USER_CTRL, 0x00, 100); // disable master

      // AK8963 magnetometer (configureAK8963): read the factory
      // sensitivity coefficients from FuseROM, then 16-bit continuous
      // 100 Hz mode.
      await this._write(AK_ADDRESS, AK_CNTL1, 0x00, 100); // power down
      await this._write(AK_ADDRESS, AK_CNTL1, 0x0f, 100); // FuseROM access mode
      const asa = await this._readBlock(AK_ADDRESS, AK_ASAX, 3);
      await this._write(AK_ADDRESS, AK_CNTL1, 0x00, 100); // power down
      await this._write(AK_ADDRESS, AK_CNTL1, (AK_BIT_16 << 4) | AK_MODE_C100HZ, 100);
      this._magCal = [...asa].map((v) => (v - 128) / 256.0 + 1.0);
    } catch (err) {
      if (retry > 1) return this._configureMpu(retry - 1);
      throw err;
    }
  }

  /*
   * The Python node's BMP280 init: chip-id check, calibration EEPROM,
   * then normal-mode x1 sampling.
   */
  async _configureBmp() {
    const chipId = await this._bus.readByte(BMP_ADDRESS, 0xd0);
    if (chipId !== BMP_CHIP_ID) {
      throw new Error(
        `BMP280 not found at 0x${BMP_ADDRESS.toString(16).toUpperCase()} ` +
        `(id register returned 0x${chipId.toString(16).toUpperCase()})`
      );
    }
    this._bmpCal = parseBmp280Calibration(await this._readBlock(BMP_ADDRESS, 0x88, 24));
    // ctrl_meas: temperature x1, pressure x1, normal mode.
    await this._write(BMP_ADDRESS, 0xf4, 0x27);
    // config: 1000 ms standby, filter off.
    await this._write(BMP_ADDRESS, 0xf5, 0xa0);
    await sleep(50);
  }

  /*
   * A failed accelerometer/magnetometer read yields zero axes, like the
   * reference driver's getDataError; a failed BMP280 read throws.
   */
  async _readAccel() {
    try {
      return convertAccel(await this._readBlock(MPU_ADDRESS, MPU_ACCEL_OUT, 6));
    } catch {
      return { x: 0, y: 0, z: 0 };
    }
  }

  async _readMag() {
    try {
      return convertMag(await this._readBlock(AK_ADDRESS, AK_MAGNET_OUT, 7), this._magCal);
    } catch {
      return { x: 0, y: 0, z: 0 };
    }
  }

  /** One reading: { roll, pitch, heading, gforce, temperature, pressure }. */
  async read() {
    const accel = await this._readAccel();
    const mag = await this._readMag();
    const angles = computeAngles(accel.x, accel.y, accel.z, mag.x, mag.y);

    const raw = await this._readBlock(BMP_ADDRESS, 0xf7, 6);
    const adcP = (raw[0] << 12) | (raw[1] << 4) | (raw[2] >> 4);
    const adcT = (raw[3] << 12) | (raw[4] << 4) | (raw[5] >> 4);
    const { temperature, pressure } = compensateBmp280(this._bmpCal, adcT, adcP);
    return { ...angles, temperature, pressure };
  }
}

module.exports = {
  IMU10DOFDriver,
  convertAccel,
  convertMag,
  computeAngles,
  parseBmp280Calibration,
  compensateBmp280,
};
