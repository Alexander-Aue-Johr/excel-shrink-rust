import test from 'node:test';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { installReportReplay, replayTime } from '../report-replay.js';

test('replay time retains hours beyond a day', () => {
  assert.equal(replayTime(25 * 3600000 + 61000), '25:01:01');
});

test('static controls load, play to the terminal checkpoint and restart', async () => {
  const nodes = [];
  const frames = [];
  const windowEvents = {};
  class Element {
    constructor(tag) { this.tag = tag; this.children = []; this.events = {}; this.attributes = {}; nodes.push(this); }
    append(...children) { this.children.push(...children); }
    setAttribute(name, value) { this.attributes[name] = value; }
    addEventListener(name, handler) { this.events[name] = handler; }
  }
  const previous = Object.fromEntries(['document', 'window', 'location', 'fetch', 'requestAnimationFrame',
    'cancelAnimationFrame', 'performance'].map(name => [name, globalThis[name]]));
  let wall = 0;
  const events = [
    { sequence: 0, at_ms: 0, state: { state: 'preparing', total: 1, completed: 0, failed: 0, reused: 0 } },
    { sequence: 1, at_ms: 1000, state: { state: 'completed', total: 1, completed: 1, failed: 0, reused: 0 } },
  ];
  const bytes = gzipSync(JSON.stringify(events));
  const manifest = { version: 1, channel: 'run-progress', complete: true,
    encoding: 'gzip-json-checkpoints', clock: 'elapsed-monotonic-milliseconds', events: 2, duration_ms: 1000,
    chunks: [{ file: 'chunk-000000.json.gz', events: 2, first_sequence: 0, first_at_ms: 0, last_at_ms: 1000,
      compressed_bytes: bytes.length, sha256: createHash('sha256').update(bytes).digest('hex') }] };
  try {
    globalThis.document = { createElement: tag => new Element(tag) };
    globalThis.window = { addEventListener: (name, callback) => { windowEvents[name] = callback; } };
    globalThis.location = { href: 'https://example.test/run/index.html' };
    globalThis.performance = { now: () => wall };
    globalThis.requestAnimationFrame = callback => { frames.push(callback); return frames.length; };
    globalThis.cancelAnimationFrame = () => {};
    globalThis.fetch = async url => new Response(new URL(url).pathname.endsWith('manifest.json')
      ? JSON.stringify(manifest) : bytes);
    installReportReplay(new Element('main'), { available: true, manifest: 'replay/manifest.json' });
    const load = nodes.find(node => node.textContent === 'Load recorded replay');
    const play = nodes.find(node => node.textContent === 'Play');
    const restart = nodes.find(node => node.textContent === 'Restart');
    const progress = nodes.find(node => node.tag === 'progress');
    const observation = nodes.find(node => node.attributes['aria-live'] === 'polite');
    await load.events.click();
    assert.equal(progress.value, 0);
    assert.equal(load.hidden, true);
    assert.equal(frames.length, 0);
    play.events.click();
    wall = 2000;
    frames.shift()(2000);
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(progress.value, 1);
    assert.match(observation.textContent, /Phase completed/);
    assert.equal(play.textContent, 'Play');
    assert.equal(frames.length, 0);
    restart.events.click();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(progress.value, 0);
    assert.match(observation.textContent, /Phase preparing/);
    windowEvents.pagehide();
  } finally {
    for (const [name, value] of Object.entries(previous)) {
      if (value === undefined) delete globalThis[name];
      else globalThis[name] = value;
    }
  }
});
