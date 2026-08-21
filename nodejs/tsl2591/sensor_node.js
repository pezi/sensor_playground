#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — TSL2591 (Node.js)
 *
 * Implements the Sensor Playground Sensor Interface on single-board
 * computers (Raspberry Pi & co.) with a TSL2591 I2C sensor (visible light,
 * infrared, illuminance).
 *
 * The chip has two photodiodes: channel 0 is broadband (visible + IR) and
 * channel 1 is infrared only. Neither is "visible light" on its own — the
 * difference of the two is. Lux is a third thing again, derived from both
 * channels together with the gain and integration time.
 *
 * - HTTPS REST API on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   Wi-Fi with a warning (use the Python node for BLE).
 *
 * Set "emulation": true in config.json to generate plausible readings
 * without the sensor hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { HTTPS_PORT, startDiscoveryListener, runRestServer } = require('../common/wifi');

const round1 = (x) => Math.round(x * 10) / 10;
const uniform = (lo, hi) => lo + Math.random() * (hi - lo);

/*
 * Emulation: a lit room near a window — a few hundred lux drifting on a
 * slow cycle, with the infrared channel holding the roughly one-third share
 * of the broadband count that daylight and incandescent lamps produce
 * (like the Python node).
 */
function emulatedRead() {
  const t = Date.now() / 1000;
  const broadband = Math.round(5400 + 1200 * Math.sin(t / 90.0) + uniform(-40, 40));
  const infrared = Math.round(broadband * 0.28 + uniform(-20, 20));
  return {
    visible: Math.max(0, broadband - infrared),
    ir: infrared,
    lux: round1(310 + 70 * Math.sin(t / 90.0) + uniform(-2, 2)),
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const i2cBus = config.i2c_bus ?? 1;
  const sslCert = config.ssl_cert || 'cert.pem';
  const sslKey = config.ssl_key || 'key.pem';

  let read;
  if (config.emulation) {
    console.log('Emulation mode: generating TSL2591 readings without hardware');
    read = async () => emulatedRead();
  } else {
    console.log(`Initializing TSL2591 sensor on /dev/i2c-${i2cBus}...`);
    const { TSL2591Driver } = require('./tsl2591_driver');
    const driver = await TSL2591Driver.create(
      i2cBus,
      config.gain ?? 'med',
      config.integration_ms ?? 300,
      config.auto_gain ?? true
    );
    read = async () => {
      const { broadband, infrared } = await driver.rawLuminosity();
      // Floored: in near-darkness noise can put IR marginally above the
      // broadband channel, and a negative amount of visible light is
      // meaningless on a chart.
      const reading = { visible: Math.max(0, broadband - infrared), ir: infrared };
      // A saturated channel makes the lux value wrong. The counts still
      // show the app that it is very bright, so omit only the lux rather
      // than publish a wrong value — the metric registry renders whatever
      // keys are present.
      const lux = driver.lux(broadband, infrared);
      if (lux !== null) reading.lux = round1(lux);
      await driver.autoGainStep(broadband);
      return reading;
    };
  }

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const readDiscovery = async () => {
    const full = await read();
    if (!full) return {};
    const short = { vis: full.visible, ir: full.ir };
    if (full.lux !== undefined) short.lux = full.lux;
    return short;
  };
  startDiscoveryListener('TSL2591', hostname, HTTPS_PORT, readDiscovery);
  runRestServer('TSL2591', config.api_key, hostname, read, sslCert, sslKey);
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
