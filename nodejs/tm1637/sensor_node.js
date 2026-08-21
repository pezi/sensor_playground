#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Clock Node — Grove 4-Digit Display / TM1637 (Node.js)
 *
 * Implements the *actuator* variant of the Sensor Playground Sensor
 * Interface: the app pushes the time to show (and a brightness), and the
 * node reports the state it is actually displaying — which keeps changing
 * on its own, because once a time is set the node advances the minute and
 * blinks the colon autonomously.
 *
 *     app -> node   {"time": "HH:MM"}     set the displayed time (24-hour)
 *                   {"brightness": 0..7}  set the brightness (clamped)
 *     node -> app   {"time": "12:34", "brightness": 3}   current state
 *                   {"time": null, "brightness": 3}      no time set yet
 *
 * The node pushes its state on connect, after every accepted command, and
 * on each minute rollover — never on the colon blink, so state traffic
 * stays at one message a minute. Before the first time set the display
 * shows "--:--".
 *
 * The node owns the state; the app renders what the node last reported
 * rather than what it asked for, so a command that never arrived cannot
 * leave the app showing a time the display does not.
 *
 * EMULATION ONLY. The TM1637 speaks a proprietary two-wire protocol that
 * has to be bit-banged: one frame is ~50 individually driven line
 * transitions on two GPIOs. Node.js has no GPIO path that can do this on
 * Debian 13 — node-libgpiod does not build against libgpiod 2.x, and the
 * gpioset CLI fallback the LED node uses spawns a process per level
 * change, which cannot carry a bus. With "emulation": false the node
 * prints an error and exits; use the Python, Go or Rust node for a real
 * display.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Usage:
 *     cp config.example.json config.json   # set "emulation": true
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');

const POLL_INTERVAL_MS = 50; // 20 Hz clock/blink tick
const COLON_BLINK_MS = 500; // colon on for 500 ms, off for 500 ms
const DEFAULT_BRIGHTNESS = 3;

// -- Display -----------------------------------------------------------------

/*
 * Prints the displayed state instead of driving hardware. Prints on
 * time/brightness changes and minute rollovers only — the colon blink
 * would flood the console at 1 Hz.
 */
class EmulatedDisplay {
  constructor() {
    this._lastPrinted = null;
  }

  /** Writes the time (or dashes when [hour] is null) to the panel. */
  render(hour, minute, colonOn, brightness) {
    const text = hour === null ? '--:--' : `${pad2(hour)}:${pad2(minute)}`;
    const shown = `${text}/${brightness}`;
    if (shown === this._lastPrinted) return;
    this._lastPrinted = shown;
    console.log(`[emulation] display [${text}] brightness=${brightness}`);
  }

  close() {}
}

function pad2(value) {
  return String(value).padStart(2, '0');
}

// -- Clock state -------------------------------------------------------------

/*
 * Owns the displayed clock state — the single source of truth this node
 * publishes — and keeps it ticking.
 *
 * Every command marks the state as pending publication, even one that
 * does not change it: a client that guessed wrong about the current state
 * would otherwise never be corrected. The colon blink renders but never
 * marks pending; the minute rollover does both.
 */
class ClockController {
  constructor(display, brightness) {
    this._display = display;
    this.hour = null;
    this.minute = null;
    this.brightness = clampBrightness(brightness);
    this._pending = true; // publish the initial state as soon as we serve
    this._colonOn = false;
    this._lastBlink = 0;
    this._minuteAccum = 0;
    this._lastTick = null;
    this._render();
  }

  _render() {
    this._display.render(this.hour, this.minute, this._colonOn, this.brightness);
  }

  /** Sets the displayed time (24-hour) and restarts the minute phase. */
  setTime(hour, minute) {
    this.hour = hour;
    this.minute = minute;
    // ":00 seconds" is now, and a lit colon gives immediate feedback.
    this._minuteAccum = 0;
    this._colonOn = true;
    this._lastBlink = Date.now();
    this._render();
    this._pending = true;
  }

  /** Sets the display brightness, clamped to 0..7. */
  setBrightness(brightness) {
    this.brightness = clampBrightness(brightness);
    this._render();
    this._pending = true;
  }

  /*
   * Advances the local clock and blinks the colon. Only the minute
   * rollover marks the state pending; the blink is render-only.
   */
  tick() {
    const now = Date.now();
    if (this._lastTick === null) this._lastTick = now;
    const elapsed = (now - this._lastTick) / 1000;
    this._lastTick = now;

    if (this.hour === null || this.minute === null) return;

    if (now - this._lastBlink >= COLON_BLINK_MS) {
      this._lastBlink = now;
      this._colonOn = !this._colonOn;
      this._render();
    }

    this._minuteAccum += elapsed;
    if (this._minuteAccum < 60.0) return;
    this._minuteAccum -= 60.0;
    this.minute += 1;
    if (this.minute >= 60) {
      this.minute = 0;
      this.hour += 1;
      if (this.hour >= 24) this.hour = 0; // midnight rollover
    }
    this._render();
    this._pending = true;
  }

  /** Returns true once after each change, clearing the pending flag. */
  takePending() {
    const pending = this._pending;
    this._pending = false;
    return pending;
  }

  /** Returns the state payload the transports publish. */
  state() {
    const time =
      this.hour === null || this.minute === null
        ? null
        : `${pad2(this.hour)}:${pad2(this.minute)}`;
    return { time, brightness: this.brightness };
  }

  close() {
    this._display.close();
  }
}

function clampBrightness(brightness) {
  return Math.max(0, Math.min(7, Math.trunc(brightness)));
}

// -- Commands ----------------------------------------------------------------

const TIME_PATTERN = /^(\d{2}):(\d{2})$/;

/** Parses an "HH:MM" string, returning [hour, minute] or null. */
function parseTimeText(text) {
  if (typeof text !== 'string') return null;
  const match = TIME_PATTERN.exec(text);
  if (match === null) return null;
  const hour = Number(match[1]);
  const minute = Number(match[2]);
  if (hour > 23 || minute > 59) return null;
  return [hour, minute];
}

/** Executes one JSON command pushed by the app over the WebSocket. */
function handleJsonCommand(clock, message) {
  let command;
  try {
    command = JSON.parse(message);
  } catch {
    console.log('Ignoring malformed command');
    return;
  }
  if (command === null || typeof command !== 'object') {
    console.log('Ignoring malformed command');
    return;
  }

  if ('time' in command) {
    const parsed = parseTimeText(command.time);
    if (parsed === null) {
      console.log('Ignoring invalid time');
      return;
    }
    const [hour, minute] = parsed;
    console.log(`Command: time ${pad2(hour)}:${pad2(minute)}`);
    clock.setTime(hour, minute);
    return;
  }

  const brightness = command.brightness;
  if (typeof brightness !== 'number' || !Number.isInteger(brightness)) {
    console.log('Ignoring unknown command');
    return;
  }
  console.log(`Command: brightness ${brightness}`);
  clock.setBrightness(brightness);
}

// -- State loop --------------------------------------------------------------

/*
 * Ticks the clock and publishes every pending state. All sources of
 * change funnel through here — a command handler only mutates the
 * controller and this loop is what puts the result on the wire, so the
 * app sees a command echo and a minute rollover the same way.
 */
function startStateLoop(clock, publish) {
  setInterval(() => {
    clock.tick();
    if (clock.takePending()) {
      const state = clock.state();
      console.log(`state: ${JSON.stringify(state)}`);
      publish(state);
    }
  }, POLL_INTERVAL_MS);
}

// -- Main --------------------------------------------------------------------

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'TM1637';

  if (!config.emulation) {
    console.log(
      'Error: the Node.js port cannot drive a TM1637. The display speaks a\n' +
      'proprietary two-wire protocol that has to be bit-banged — one frame is\n' +
      '~50 individually driven line transitions on two GPIOs — and Node.js has\n' +
      'no GPIO path for that on Debian 13 (node-libgpiod does not build against\n' +
      'libgpiod 2.x, and the gpioset CLI spawns a process per level change).\n' +
      'Use the Python, Go or Rust node for a real display, or set\n' +
      '"emulation": true in config.json to drive a virtual one.'
    );
    process.exit(1);
  }
  console.log('Emulation mode: driving a virtual display without hardware');

  const clock = new ClockController(new EmulatedDisplay(),
    config.brightness === undefined ? DEFAULT_BRIGHTNESS : config.brightness);

  const shutdown = () => {
    clock.close();
    console.log('Stopped.');
    process.exit(0);
  };
  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(
    config.api_key,
    (send) => send(clock.state()),
    (message) => handleJsonCommand(clock, message)
  );
  startDiscoveryListener(sensorName, hostname, WS_PORT, null);
  startStateLoop(clock, broadcast);
}

main();
