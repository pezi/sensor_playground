#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Sensor Node — Digital Contact Sensor (Node.js, push)
 *
 * One generic *push* node for the simple two-state Grove/BakeBit digital
 * sensors that react to an event:
 *
 *     BUTTON     — Grove/BakeBit button      (pressed)
 *     HALL       — Grove Hall sensor         (magnetic field present)
 *     MAGSWITCH  — Grove magnetic switch     (reed switch closed)
 *     PIR        — Grove PIR motion sensor   (motion detected)
 *     VIBRATION  — Grove vibration sensor    (SW-420, vibration)
 *     LINEFINDER — Grove Line Finder         (dark line under the sensor)
 *
 * The node polls the input (debounced) and pushes one JSON message
 * whenever the state changes:
 *
 *     {"active": true}    sensor triggered
 *     {"active": false}   sensor released
 *
 * The input is read through libgpiod ("interface": "gpio", via the
 * optional node-libgpiod package — /sys/class/gpio is gone in Debian 13).
 * Bias resistors are NOT configured on that path (the library does not
 * expose them): a sensor whose released state floats — the reed-based
 * Grove Magnetic Switch — needs an external pull resistor. On libgpiod
 * 2.x systems (Debian 13, where node-libgpiod cannot build) the node
 * falls back to polling the gpioget CLI at ~5 Hz, which does configure
 * the bias but adds latency — see the README. The Arduino-based
 * extension hats the Python node also supports ("interface": "hat") are
 * not implemented in this port.
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

const POLL_INTERVAL_MS = 20; // 50 Hz input poll
const DEBOUNCE_MS = 30; // line must be stable this long before a change counts
const CLI_POLL_INTERVAL_MS = 200; // gpioget fallback: one process spawn per poll

// -- Hardware ----------------------------------------------------------------

/*
 * Builds the configured input source: { readActive, pollIntervalMs,
 * close }. readActive() returns true while the sensor is triggered.
 */
function makeHardware(config) {
  if (config.emulation) {
    // The contact toggles roughly every five seconds, as if someone
    // slowly pressed and released a button.
    return {
      readActive: () => Math.floor(Date.now() / 5000) % 2 === 0,
      pollIntervalMs: POLL_INTERVAL_MS,
      close: () => {},
    };
  }
  if ((config.interface || 'gpio') !== 'gpio') {
    console.log(`Error: interface "${config.interface}" is not supported in the Node.js port (only "gpio"; use the Python node for Arduino-based hats)`);
    process.exit(1);
  }
  if (config.pin === undefined || config.pin === null) {
    console.log('Error: config.json: pin is required');
    process.exit(1);
  }

  const chipName = (config.gpio_chip || '/dev/gpiochip0').replace('/dev/', '');
  const activeLow = config.active_low !== false;

  // Preferred: node-libgpiod (needs libgpiod 1.x, e.g. Debian 12).
  try {
    const gpiod = require('node-libgpiod');
    const chip = new gpiod.Chip(chipName);
    const line = chip.getLine(config.pin);
    line.requestInputMode('sensor-playground-contact');
    return {
      readActive: () => {
        const high = line.getValue() === 1;
        return activeLow ? !high : high;
      },
      pollIntervalMs: POLL_INTERVAL_MS,
      close: () => line.release(),
    };
  } catch {
    // Fallback for libgpiod 2.x systems (Debian 13, where node-libgpiod
    // cannot build): poll the gpioget CLI. One process spawn per sample
    // limits the rate to ~5 Hz — fine for reed/PIR/hall contacts, too
    // slow for quick button taps (see the README).
    console.log('node-libgpiod unavailable, polling the input via the gpioget CLI (~5 Hz)');
    return makeGpiogetInput(chipName, config.pin, activeLow);
  }
}

/*
 * Input through libgpiod 2.x's gpioget, refreshed asynchronously on its
 * own timer; readActive() returns the last sample. Unlike the
 * node-libgpiod path this one does configure the internal bias (gpioget
 * --bias), matching the Python node.
 */
function makeGpiogetInput(chipName, pin, activeLow) {
  const { execFile } = require('child_process');
  const args = [
    '--numeric',
    '--bias', activeLow ? 'pull-up' : 'pull-down',
    '-c', chipName,
    String(pin),
  ];
  let active = false;
  let inFlight = false;

  const poll = () => {
    if (inFlight) return;
    inFlight = true;
    execFile('gpioget', args, (err, stdout) => {
      inFlight = false;
      if (err) return;
      const high = stdout.trim() === '1';
      active = activeLow ? !high : high;
    });
  };
  // Verify the tool exists up front so a missing gpiod package fails loudly.
  execFile('gpioget', ['--version'], (err) => {
    if (err) {
      console.log('Error: gpioget not found - install the gpiod package (sudo apt install gpiod)');
      process.exit(1);
    }
  });
  const timer = setInterval(poll, CLI_POLL_INTERVAL_MS);
  poll();
  return {
    readActive: () => active,
    pollIntervalMs: CLI_POLL_INTERVAL_MS,
    close: () => clearInterval(timer),
  };
}

// -- Input loop --------------------------------------------------------------

// Latest published state, shared with new clients (null until the first
// debounced reading).
const state = { active: null };

/** Polls the input (debounced) and pushes each state change. */
function startInputLoop(hardware, publish) {
  let stable = null; // last published active state
  let candidate = null; // pending state awaiting debounce
  let candidateSince = 0;

  setInterval(() => {
    const active = hardware.readActive();
    const now = Date.now();
    if (active !== candidate) {
      candidate = active;
      candidateSince = now;
    } else if (active !== stable && now - candidateSince >= DEBOUNCE_MS) {
      stable = active;
      state.active = active;
      console.log(`active: ${active}`);
      publish({ active });
    }
  }, hardware.pollIntervalMs);
}

// -- Main --------------------------------------------------------------------

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'BUTTON';

  if (config.emulation) {
    console.log(`Emulation mode: generating ${sensorName} readings without hardware`);
  } else {
    console.log(`Initializing ${sensorName} digital contact sensor...`);
  }
  const hardware = makeHardware(config);

  const shutdown = () => {
    hardware.close();
    console.log('Stopped.');
    process.exit(0);
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  // Send the current state to a client right after it connects.
  const { broadcast } = startWsPushServer(
    config.api_key,
    (send) => {
      if (state.active !== null) send({ active: state.active });
    },
    null
  );
  startDiscoveryListener(sensorName, hostname, WS_PORT, null);
  startInputLoop(hardware, broadcast);
}

main();
