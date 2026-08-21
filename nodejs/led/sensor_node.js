#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground LED Node — LED + optional push button (Node.js)
 *
 * Implements the *actuator* variant of the Sensor Playground Sensor
 * Interface: the app sends a switch command and the node reports the
 * resulting state back, because the LED can also be toggled by a push
 * button wired to the node itself.
 *
 *     app -> node   {"led": true}     switch on
 *                   {"led": false}    switch off
 *                   {"toggle": true}  flip
 *     node -> app   {"led": true|false}   current state (on connect and
 *                                         after every change)
 *
 * The node owns the state; the app renders what the node last reported.
 *
 * The LED and button are driven through libgpiod ("interface": "gpio",
 * via the optional node-libgpiod package — /sys/class/gpio is gone in
 * Debian 13). Bias resistors are NOT configured by this port (the library
 * does not expose them): with "button_active_low": true wire an external
 * pull-up, or rely on the line's default bias. The Arduino-based
 * extension hats the Python node also supports ("interface": "hat") are
 * not implemented in this port.
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * Set "button_pin": null for an LED-only node, and "emulation": true to
 * run without any hardware at all.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');

const POLL_INTERVAL_MS = 20; // 50 Hz button poll
const DEBOUNCE_MS = 30; // line must be stable this long before a press counts

// -- Hardware ----------------------------------------------------------------

function makeHardware(config) {
  if (config.emulation) {
    return {
      write: (on) => console.log(`[emulation] LED ${on ? 'on' : 'off'}`),
      readPressed: () => false,
      close: () => {},
    };
  }
  if ((config.interface || 'gpio') !== 'gpio') {
    console.log(`Error: interface "${config.interface}" is not supported in the Node.js port (only "gpio"; use the Python node for Arduino-based hats)`);
    process.exit(1);
  }
  if (config.led_pin === undefined || config.led_pin === null) {
    console.log('Error: config.json: led_pin is required');
    process.exit(1);
  }

  const chipName = (config.gpio_chip || '/dev/gpiochip0').replace('/dev/', '');
  const ledActiveLow = config.led_active_low === true;

  // Preferred: node-libgpiod (needs libgpiod 1.x, e.g. Debian 12).
  try {
    const gpiod = require('node-libgpiod');
    const chip = new gpiod.Chip(chipName);
    const ledLine = chip.getLine(config.led_pin);
    ledLine.requestOutputMode('sensor-playground-led');
    const write = (on) => {
      const high = ledActiveLow ? !on : on;
      ledLine.setValue(high ? 1 : 0);
    };

    let readPressed = () => false;
    let buttonLine = null;
    if (config.button_pin !== undefined && config.button_pin !== null) {
      const buttonActiveLow = config.button_active_low !== false;
      buttonLine = chip.getLine(config.button_pin);
      buttonLine.requestInputMode('sensor-playground-button');
      readPressed = () => {
        const high = buttonLine.getValue() === 1;
        return buttonActiveLow ? !high : high;
      };
    }
    return {
      write,
      readPressed,
      close: () => {
        ledLine.release();
        if (buttonLine) buttonLine.release();
      },
    };
  } catch {
    // Fallback for libgpiod 2.x systems (Debian 13, where node-libgpiod
    // cannot build): drive the LED with the gpioset CLI, one daemonized
    // holder process per state (killed and respawned on every switch).
    // The button is not supported on this path.
    console.log('node-libgpiod unavailable, driving the LED via the gpioset CLI');
    if (config.button_pin !== undefined && config.button_pin !== null) {
      console.log('Warning: the button is not supported via gpioset; ignoring button_pin');
    }
    return makeGpiosetOutput(chipName, config.led_pin, ledActiveLow);
  }
}

/*
 * LED output through libgpiod 2.x's gpioset: `gpioset -z` claims the line
 * and holds it until killed, so every switch kills the previous holder and
 * spawns a new one. Switches are serialized through a promise chain — the
 * old holder must have released the line before the next may claim it.
 */
function makeGpiosetOutput(chipName, pin, activeLow) {
  const { execFile, spawn } = require('child_process');
  let holder = null;
  let queue = Promise.resolve();

  const write = (on) => {
    const high = activeLow ? !on : on;
    queue = queue.then(() => new Promise((resolve) => {
      const previous = holder;
      const claim = () => {
        holder = spawn('gpioset', ['-z', '-c', chipName, `${pin}=${high ? 1 : 0}`],
          { stdio: 'ignore' });
        setTimeout(resolve, 30); // let it claim the line before the next switch
      };
      if (previous && previous.exitCode === null) {
        previous.once('exit', claim);
        previous.kill('SIGTERM');
      } else {
        claim();
      }
    }));
  };
  // Verify the tool exists up front so a missing gpiod package fails loudly.
  execFile('gpioset', ['--version'], (err) => {
    if (err) {
      console.log('Error: gpioset not found - install the gpiod package (sudo apt install gpiod)');
      process.exit(1);
    }
  });
  return {
    write,
    readPressed: () => false,
    close: () => {
      if (holder && holder.exitCode === null) holder.kill('SIGTERM');
    },
  };
}

// -- LED state ---------------------------------------------------------------

/*
 * Owns the LED state — the single source of truth this node publishes.
 * Every switch marks the state as pending publication, even one that does
 * not change it: a client that guessed wrong about the current state
 * would otherwise never be corrected.
 */
class LedController {
  constructor(write) {
    this._write = write;
    this.on = false;
    this._pending = true; // publish the initial state as soon as we serve
    this._write(false);
  }

  set(on) {
    this.on = on;
    this._write(on);
    this._pending = true;
  }

  toggle() {
    this.set(!this.on);
  }

  takePending() {
    const pending = this._pending;
    this._pending = false;
    return pending;
  }
}

/** Executes one JSON command pushed by the app over the WebSocket. */
function handleJsonCommand(led, message) {
  let command;
  try {
    command = JSON.parse(message);
  } catch {
    console.log('Ignoring malformed command');
    return;
  }
  if (command.toggle === true) {
    console.log('Command: toggle');
    led.toggle();
    return;
  }
  if (typeof command.led !== 'boolean') {
    console.log("Ignoring command without a boolean 'led'");
    return;
  }
  console.log(`Command: led ${command.led ? 'on' : 'off'}`);
  led.set(command.led);
}

/*
 * Polls the button (debounced) and publishes every state change. Both
 * sources of change funnel through here — a command handler only mutates
 * the controller, and this loop puts the result on the wire, so the app
 * sees an app-initiated switch and a button press the same way.
 */
function startStateLoop(led, readPressed, publish) {
  let pressed = null; // last debounced button state
  let candidate = null; // pending state awaiting debounce
  let candidateSince = 0;

  setInterval(() => {
    const now = Date.now();
    const isPressed = readPressed();
    if (isPressed !== candidate) {
      candidate = isPressed;
      candidateSince = now;
    } else if (isPressed !== pressed && now - candidateSince >= DEBOUNCE_MS) {
      const firstReading = pressed === null;
      pressed = isPressed;
      // The first stable reading only establishes the idle level; toggle
      // on press, not on release, so one press is one toggle.
      if (isPressed && !firstReading) {
        console.log('Button pressed: toggling LED');
        led.toggle();
      }
    }

    if (led.takePending()) {
      console.log(`led: ${led.on}`);
      publish({ led: led.on });
    }
  }, POLL_INTERVAL_MS);
}

// -- Main --------------------------------------------------------------------

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'LED';

  if (config.emulation) {
    console.log('Emulation mode: switching a virtual LED without hardware');
  } else {
    console.log('Initializing LED node...');
  }
  const hardware = makeHardware(config);
  const led = new LedController(hardware.write);

  // Leave the LED dark rather than stuck on after the node exits.
  const shutdown = () => {
    led.set(false);
    hardware.close();
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
    (send) => send({ led: led.on }),
    (message) => handleJsonCommand(led, message)
  );
  startDiscoveryListener(sensorName, hostname, WS_PORT, null);
  startStateLoop(led, hardware.readPressed, broadcast);
}

main();
