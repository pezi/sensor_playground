#!/usr/bin/env node
'use strict';
/*
 * Sensor Playground Speaker Node — Grove Speaker (Node.js)
 *
 * Implements the *actuator* variant of the Sensor Playground Sensor
 * Interface with a Grove Speaker — a small amplified loudspeaker on a
 * digital pin. Like the LED the node talks in both directions: the app
 * asks for a tone, the built-in melody, or silence, and the node reports
 * what is *actually* sounding — a tone ends on its own when its duration
 * runs out, so the app follows the node's reports rather than its own taps.
 *
 *     app -> node   {"tone": {"freq": 440, "ms": 400}}   play one tone
 *                   {"melody": true}                     play the built-in melody
 *                   {"stop": true}                       silence
 *     node -> app   {"freq": 440} / {"freq": 0}          what is sounding
 *
 * - WebSocket server (ws://) on port 9132 + UDP discovery on port 9133
 * - BLE is not supported in this port; "transport": "ble" falls back to
 *   the WebSocket with a warning (use the Python or Rust node for BLE).
 *
 * EMULATION ONLY on real hardware: sounding a tone means toggling a GPIO
 * line up to 20000 times a second, and Node's event loop cannot hold that
 * pace (the same reason dht11, sht11, ppd42ns, ultrasonic and tm1637 are
 * emulation-only here). The protocol, the state machine and the timing of
 * tones and melodies are complete — only the pin is not driven. Use the
 * Python, Rust or Go node to actually make a sound.
 *
 * Usage:
 *     cp config.example.json config.json   # edit with your settings
 *     node sensor_node.js
 */

const { loadConfig, hostnameOr } = require('../common/config');
const { WS_PORT, startDiscoveryListener } = require('../common/wifi');
const { startWsPushServer } = require('../common/ws_server');
const { SpeakerController, handleJSONCommand } = require('./speaker');

// How often the playback loop advances tones and publishes state changes.
const POLL_INTERVAL_MS = 20;

/* Prints the sounding state instead of driving hardware. */
const emulatedOutput = {
  play(frequency) {
    console.log(`[emulation] speaker ${frequency > 0 ? `${frequency} Hz` : 'silent'}`);
  },
  close() {},
};

function main() {
  const config = loadConfig(__dirname);
  const hostname = hostnameOr(config.hostname);
  const sensorName = config.sensor_name || 'SPEAKER';

  if (!config.emulation) {
    console.log(
      'Warning: this port cannot drive the speaker pin — Node cannot toggle a\n' +
      'GPIO line at audio rates. Running in emulation mode; use the Python,\n' +
      'Rust or Go node to actually sound tones.'
    );
  }
  console.log('Emulation mode: printing tones without hardware');
  const speaker = new SpeakerController(emulatedOutput);

  if ((config.transport || 'wifi') === 'ble') {
    console.log('Warning: BLE transport is not supported in the Node.js port, using wifi.');
  }

  const { broadcast } = startWsPushServer(
    config.api_key,
    (send) => send(speaker.state()),
    (message) => handleJSONCommand(speaker, message)
  );
  startDiscoveryListener(sensorName, hostname, WS_PORT, null);

  // Command handlers only mutate the controller; this loop ends tones on
  // time and puts the resulting states on the wire, so the app sees a
  // command echo and a tone that ran out the same way.
  setInterval(() => {
    speaker.tick();
    if (speaker.takePending()) broadcast(speaker.state());
  }, POLL_INTERVAL_MS);
}

try {
  main();
} catch (err) {
  console.log(`Error: ${err.message}`);
  process.exit(1);
}
