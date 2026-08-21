#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Grove Rotary Angle Sensor (Node.js, push)
 *
 * Reads a Grove Rotary Angle Sensor — a 10 kOhm potentiometer with 300°
 * of mechanical travel — through the Seeed Grove Base Hat's 12-bit ADC
 * and reports the knob position.
 * https://wiki.seeedstudio.com/Grove-Rotary_Angle_Sensor/
 *
 * Unlike the light sensor (also analog, but polled over HTTPS every few
 * seconds) this is a *push* node: the app draws a needle that tracks the
 * knob, so a reading that is seconds old is useless. The node samples the
 * ADC continuously and sends a message whenever the knob has moved
 * further than the deadband, plus the current position once per client
 * connect:
 *
 *     {"adc": 2048, "adcMax": 4095, "angle": 150.1, "angleMax": 300.0}
 *
 * adcMax travels with every message because the converter's width belongs
 * to the board doing the reading, not to the knob: the Grove Base Hat is
 * 12-bit (0-4095), the Arduino-based hats the Python node also supports
 * ("nano", "grovePlus", both 10-bit) are not implemented in this port.
 * `angle`/`angleMax` carry the same position expressed in degrees, so the
 * app never needs to know the knob's mechanical travel either.
 *
 * The Raspberry Pi has no analog input, so an analog sensor needs an
 * extension hat with an ADC — there is no direct-GPIO option (unlike the
 * digital contact node).
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Set "emulation": true in config.json to generate plausible readings
 * without the sensor hardware.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');

const POLL_INTERVAL_MS = 40; // 25 Hz — matches the ESP32 sketch's publish ceiling

const HAT_I2C_ADDRESS = 0x04; // Grove Base Hat (STM32F030 ADC)
const HAT_ADC_BASE = 0x10; // raw 12-bit value registers, one per channel

// Full-scale ADC count per hat, keyed by hat_type. Only "grove" can be
// read by this port, but the emulation reports the range the configured
// hat really would, like the Python node.
const ADC_MAX_BY_HAT = {
  grove: 4095, // Grove Base Hat, 12-bit STM32F030 ADC
  nano: 1023, // NanoHat Hub, 10-bit AVR analogRead (BakeBit)
  grovePlus: 1023, // GrovePi+, 10-bit ATMEGA328P analogRead
};

// Mechanical travel of the knob, end to end. 300° for the Grove sensor.
const DEFAULT_ANGLE_MAX = 300.0;

// Default deadband as a fraction of full scale: how far the count must
// move before a new message goes out. ADC noise jitters the reading by a
// few counts with the knob at rest, which would otherwise flood the link.
const DEFAULT_DEADBAND_RATIO = 0.006; // ~24 counts of 4095

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Builds the position message. The scale travels with every reading. */
function payloadFor(adc, adcMax, angleMax) {
  return {
    adc,
    adcMax,
    angle: Math.round((adc / adcMax) * angleMax * 10) / 10,
    angleMax,
  };
}

async function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const hatType = config.hat_type || 'grove';
  const pin = config.pin ?? 0;
  const i2cBus = config.i2c_bus ?? 1;
  const angleMax = config.angle_max ?? DEFAULT_ANGLE_MAX;

  let adcMax;
  let readAdc; // returns the raw count, clamped to 0..adcMax
  if (config.emulation) {
    console.log('Emulation mode: generating ROTARY readings without hardware');
    // Sweeps slowly from end to end and back, as if someone were turning
    // the knob by hand, so the app's needle has something to follow. The
    // emulation reports the range the configured hat would really report,
    // so the app is exercised against a 10-bit node as easily as a 12-bit
    // one.
    adcMax = ADC_MAX_BY_HAT[hatType] ?? 4095;
    readAdc = async () => {
      // A 20-second triangle wave over the full span.
      const phase = ((Date.now() / 1000) % 20.0) / 20.0;
      const fraction = 1 - Math.abs(2 * phase - 1);
      return Math.round(fraction * adcMax);
    };
  } else {
    if (hatType !== 'grove') {
      console.log(`Error: hat_type "${hatType}" is not supported in the Node.js port (only "grove"; use the Python node for Arduino-based hats)`);
      process.exit(1);
    }
    if (pin < 0 || pin > 7) {
      console.log(`Error: invalid channel ${pin} - valid range [0,7]`);
      process.exit(1);
    }
    console.log(`Initializing Grove Rotary Angle Sensor on grove hat, channel ${pin}...`);
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
    const bus = await i2c.openPromisified(i2cBus);
    adcMax = ADC_MAX_BY_HAT.grove;
    readAdc = async () => {
      // The raw 12-bit ADC value; SMBus words are little-endian.
      const value = await bus.readWord(HAT_I2C_ADDRESS, HAT_ADC_BASE + pin);
      return Math.max(0, Math.min(adcMax, value));
    };
  }

  console.log(`ADC range: 0-${adcMax}, travel ${angleMax.toFixed(0)} deg`);

  // A deadband given in counts wins; otherwise scale it to this hat's
  // range so a 10-bit node is not held to a 12-bit node's precision.
  const deadband = config.deadband ?? Math.max(1, Math.ceil(adcMax * DEFAULT_DEADBAND_RATIO));

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  let lastAdc = null; // latest published count, shared with new clients

  // Send the current position to a client right after it connects.
  const { broadcast } = startWsPushServer(config.api_key, (send) => {
    if (lastAdc !== null) {
      send(payloadFor(lastAdc, adcMax, angleMax));
    } else {
      readAdc()
        .then((adc) => send(payloadFor(adc, adcMax, angleMax)))
        .catch(() => {});
    }
  }, null);
  startDiscoveryListener('ROTARY', hostname, WS_PORT, null);

  // Samples the knob and publishes whenever it moves past the deadband.
  let published = null;
  for (;;) {
    try {
      const adc = await readAdc();
      const moved = published === null || Math.abs(adc - published) >= deadband;
      // Pin the ends too, so a knob turned fully reports exactly 0 or
      // full scale instead of stopping a deadband short of it.
      const atEnd = (adc === 0 || adc === adcMax) && adc !== published;
      if (moved || atEnd) {
        published = adc;
        lastAdc = adc;
        const payload = payloadFor(adc, adcMax, angleMax);
        console.log(`adc: ${adc}/${adcMax}  angle: ${payload.angle}`);
        broadcast(payload);
      }
    } catch (err) {
      console.log(`Sensor read failed: ${err.message}`);
    }
    await sleep(POLL_INTERVAL_MS);
  }
}

main().catch((err) => {
  console.log(`Error: ${err.message}`);
  process.exit(1);
});
