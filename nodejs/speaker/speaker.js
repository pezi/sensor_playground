'use strict';
/*
 * Playback state for the speaker node — the single source of truth this
 * node publishes, and the JSON command handler that stages it.
 *
 * Command handlers only stage playback; tick() — driven by the serving
 * loop — is what advances melodies and ends tones on time, and every
 * change marks the state as pending publication.
 *
 * Kept apart from sensor_node.js so the state machine can be unit tested
 * without opening a socket.
 */

// Accepted tone range; anything else is ignored as noise.
const MIN_FREQ_HZ = 20;
const MAX_FREQ_HZ = 20000;

// The built-in melody: a little C-major fanfare, [freq Hz, duration ms].
const MELODY = [
  [262, 250], [330, 250], [392, 250], [523, 350],
  [392, 250], [330, 250], [262, 500],
];

class SpeakerController {
  /*
   * `output` sounds a frequency (0 = silent); `clock` returns a monotonic
   * time in milliseconds and is injectable so tests need no timers.
   */
  constructor(output, clock = () => Date.now()) {
    this._output = output;
    this._clock = clock;
    this._frequency = 0;
    this._pending = true; // publish the initial state as soon as we serve
    this._deadline = null;
    this._melody = []; // remaining [freq, ms] steps
    this._output.play(0);
  }

  get frequency() {
    return this._frequency;
  }

  /* Start one note. */
  _apply(frequency, milliseconds) {
    this._frequency = frequency;
    this._deadline = frequency > 0 ? this._clock() + milliseconds : null;
    this._output.play(frequency);
    // Publish unconditionally: a redundant command from a client that
    // guessed wrong would otherwise never be corrected.
    this._pending = true;
  }

  /* Play one tone, cancelling any melody. */
  playTone(frequency, milliseconds) {
    if (frequency < MIN_FREQ_HZ || frequency > MAX_FREQ_HZ || milliseconds <= 0) {
      console.log(`Ignoring tone ${frequency} Hz / ${milliseconds} ms`);
      return;
    }
    console.log(`Tone: ${frequency} Hz for ${milliseconds} ms`);
    this._melody = [];
    this._apply(frequency, milliseconds);
  }

  /* Start the built-in melody from its first note. */
  playMelody() {
    console.log('Melody');
    this._melody = MELODY.slice(1);
    this._apply(MELODY[0][0], MELODY[0][1]);
  }

  /* Silence the speaker, whatever it is playing. */
  stop() {
    console.log('Stop');
    this._melody = [];
    this._apply(0, 0);
  }

  /* End a finished tone, or step through the melody. */
  tick() {
    if (this._deadline === null || this._clock() < this._deadline) return;
    if (this._melody.length > 0) {
      const [frequency, milliseconds] = this._melody.shift();
      this._apply(frequency, milliseconds);
      return;
    }
    this._apply(0, 0);
  }

  /* True once after each change, clearing the pending flag. */
  takePending() {
    const pending = this._pending;
    this._pending = false;
    return pending;
  }

  /* The state payload the transports publish. */
  state() {
    return { freq: this._frequency };
  }

  close() {
    this._output.close();
  }
}

/* Execute one JSON command pushed by the app over the WebSocket. */
function handleJSONCommand(speaker, message) {
  let command;
  try {
    command = JSON.parse(message);
  } catch {
    console.log('Ignoring malformed command');
    return;
  }
  if (command === null || typeof command !== 'object') {
    console.log('Ignoring unknown command');
    return;
  }

  if (command.stop === true) {
    speaker.stop();
    return;
  }
  if (command.melody === true) {
    speaker.playMelody();
    return;
  }
  const tone = command.tone;
  if (tone !== null && typeof tone === 'object') {
    // Integers only, like the Python node's isinstance(int) check.
    if (Number.isInteger(tone.freq) && Number.isInteger(tone.ms)) {
      speaker.playTone(tone.freq, tone.ms);
      return;
    }
  }
  console.log('Ignoring unknown command');
}

module.exports = { SpeakerController, handleJSONCommand, MELODY, MIN_FREQ_HZ, MAX_FREQ_HZ };
