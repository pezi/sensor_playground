#!/usr/bin/env node
'use strict';
/*
 * Unit test: the command handling and playback state must match the Python
 * node's — out-of-range tones are ignored, the melody steps through its
 * notes, and every accepted command marks the state for publication.
 *
 * Run: node test_driver.js
 */

const assert = require('assert');

const { SpeakerController, handleJSONCommand, MELODY } = require('./speaker');

// A controller on a clock the test advances by hand, so no timers are
// needed to expire a tone.
function newTestSpeaker() {
  const played = [];
  let now = 0;
  const speaker = new SpeakerController(
    { play: (frequency) => played.push(frequency), close() {} },
    () => now
  );
  speaker.takePending(); // drop the initial state
  return { speaker, played, advance: (ms) => { now += ms; } };
}

// An accepted tone sounds, is published, and expires on its own.
{
  const { speaker, played, advance } = newTestSpeaker();
  handleJSONCommand(speaker, '{"tone": {"freq": 440, "ms": 400}}');
  assert.deepStrictEqual(speaker.state(), { freq: 440 });
  assert.strictEqual(speaker.takePending(), true, 'an accepted tone must be published');
  assert.strictEqual(played[played.length - 1], 440);

  advance(399);
  speaker.tick();
  assert.deepStrictEqual(speaker.state(), { freq: 440 }, 'the tone must last its full duration');
  advance(1);
  speaker.tick();
  assert.deepStrictEqual(speaker.state(), { freq: 0 }, 'the tone must end on time');
  assert.strictEqual(speaker.takePending(), true, 'the end of a tone must be published');
}

// Anything outside the audible range, or malformed, is ignored silently.
for (const command of [
  '{"tone": {"freq": 19, "ms": 400}}',
  '{"tone": {"freq": 20001, "ms": 400}}',
  '{"tone": {"freq": 440, "ms": 0}}',
  '{"tone": {"freq": 440.5, "ms": 400}}',
  '{"tone": {"freq": 440}}',
  '{"nonsense": true}',
  'not json',
]) {
  const { speaker } = newTestSpeaker();
  handleJSONCommand(speaker, command);
  assert.deepStrictEqual(speaker.state(), { freq: 0 }, `${command} must be ignored`);
  assert.strictEqual(speaker.takePending(), false, `${command} must not be published`);
}

// The melody walks its notes and falls silent at the end.
{
  const { speaker, advance } = newTestSpeaker();
  handleJSONCommand(speaker, '{"melody": true}');
  assert.deepStrictEqual(speaker.state(), { freq: MELODY[0][0] });
  for (const [frequency] of MELODY.slice(1)) {
    advance(1000);
    speaker.tick();
    assert.deepStrictEqual(speaker.state(), { freq: frequency });
  }
  advance(1000);
  speaker.tick();
  assert.deepStrictEqual(speaker.state(), { freq: 0 }, 'the melody must end silent');
  speaker.tick();
  assert.deepStrictEqual(speaker.state(), { freq: 0 }, 'silence must not restart anything');
}

// Stop cancels a running melody rather than letting it resume.
{
  const { speaker, advance } = newTestSpeaker();
  handleJSONCommand(speaker, '{"melody": true}');
  handleJSONCommand(speaker, '{"stop": true}');
  assert.deepStrictEqual(speaker.state(), { freq: 0 });
  assert.strictEqual(speaker.takePending(), true, 'stop must be published');
  advance(1000);
  speaker.tick();
  assert.deepStrictEqual(speaker.state(), { freq: 0 }, 'a stopped melody must not resume');
}

console.log('OK: tone validation, melody stepping and stop match the Python node');
