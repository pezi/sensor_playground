'use strict';
/*
 * Sensor layer: real BME680 and emulated variant, both feeding the same
 * IAQ calculation. Mirrors python/bme680/sensor_node.py.
 */

const GAS_BURN_IN = 50;
const HUMIDITY_BASELINE = 40.0;
const HUMIDITY_WEIGHT = 0.25;

const round1 = (x) => Math.round(x * 10) / 10;
const round2 = (x) => Math.round(x * 100) / 100;

/*
 * Rolling-baseline IAQ score ported from the dart_periphery BME680 driver
 * (same algorithm as the Python/ESP32 nodes): a 50-reading gas-resistance
 * window pre-filled with zeros (the score stabilizes only after ~50
 * readings), gas weighted 75%, humidity 25%.
 */
class IaqCalculator {
  constructor() {
    this._gasData = new Array(GAS_BURN_IN).fill(0);
    this._next = 0;
    this._lastIaq = 0;
  }

  calculate(gasResistance, humidity) {
    this._gasData[this._next] = gasResistance;
    this._next = (this._next + 1) % GAS_BURN_IN;

    const gasBaseline = Math.round(
      this._gasData.reduce((a, b) => a + b, 0) / GAS_BURN_IN
    );
    const gasOffset = gasBaseline - gasResistance;
    const humOffset = humidity - HUMIDITY_BASELINE;

    let humScore;
    if (humOffset > 0) {
      humScore = ((100.0 - HUMIDITY_BASELINE - humOffset) /
        (100.0 - HUMIDITY_BASELINE)) * (HUMIDITY_WEIGHT * 100.0);
    } else {
      humScore = ((HUMIDITY_BASELINE + humOffset) / HUMIDITY_BASELINE) *
        (HUMIDITY_WEIGHT * 100.0);
    }

    const gasWeight = 100.0 - HUMIDITY_WEIGHT * 100.0;
    let gasScore;
    if (gasOffset > 0) {
      if (gasBaseline === 0) {
        return this._lastIaq; // matches the Python ZeroDivisionError fallback
      }
      gasScore = (gasResistance / gasBaseline) * gasWeight;
    } else {
      gasScore = gasWeight;
    }

    this._lastIaq = Math.round(humScore + gasScore);
    return this._lastIaq;
  }

  current() {
    return this._lastIaq;
  }
}

/** Reads temperature, humidity, pressure and IAQ from a real BME680. */
class BME680Sensor {
  static async create(i2cBus) {
    const { BME680Driver } = require('./bme680_driver');
    const sensor = new BME680Sensor();
    sensor._driver = await BME680Driver.create(i2cBus);
    return sensor;
  }

  constructor() {
    this.name = 'BME680';
    this._iaq = new IaqCalculator();
    this._driver = null;
    this._readQueue = Promise.resolve();
  }

  /** Full-key readings for the REST API, or null on a failed read. */
  read() {
    const result = this._readQueue.then(() => this._readOnce());
    this._readQueue = result.catch(() => {});
    return result;
  }

  async _readOnce() {
    const r = await this._driver.read();
    if (r === null) return null;
    const iaq = r.gasValid && r.heatStable
      ? this._iaq.calculate(Math.trunc(r.gasResistance), r.humidity)
      : this._iaq.current();
    return {
      temperature: round1(r.temperature),
      humidity: round1(r.humidity),
      pressure: round2(r.pressure),
      iaq,
    };
  }
}

/*
 * Generates plausible BME680 readings without hardware: slow sines for
 * temperature/humidity/pressure and a bounded random walk around 120 kOhm
 * for the gas resistance, scored with the real IAQ algorithm.
 */
class EmulatedBME680Sensor {
  constructor() {
    this.name = 'BME680';
    this._iaq = new IaqCalculator();
    this._gas = 120000.0;
  }

  async read() {
    const t = Date.now() / 1000;
    this._gas = Math.min(Math.max(this._gas + uniform(-2000, 2000), 20000.0), 500000.0);
    const humidity = 45.0 + 8.0 * Math.sin(t / 97.0) + uniform(-0.5, 0.5);
    return {
      temperature: round1(22.0 + 2.0 * Math.sin(t / 60.0) + uniform(-0.1, 0.1)),
      humidity: round1(humidity),
      pressure: round2(1013.0 + 3.0 * Math.sin(t / 300.0) + uniform(-0.2, 0.2)),
      iaq: this._iaq.calculate(Math.trunc(this._gas), humidity),
    };
  }
}

function uniform(lo, hi) {
  return lo + Math.random() * (hi - lo);
}

module.exports = { BME680Sensor, EmulatedBME680Sensor };
