import test from 'node:test';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { createHash } from 'node:crypto';
import { ReplayClock, ReplayReader, validateReplayManifest } from '../replay-engine.js';

function fixture() {
  const files = new Map();
  const chunks = [];
  for (let index = 0; index < 4; index++) {
    const events = Array.from({ length: 32 }, (_, offset) => {
      const sequence = index * 32 + offset;
      return { sequence, at_ms: sequence * 10 + 5, state: { completed: sequence } };
    });
    const file = `chunk-${String(index).padStart(6, '0')}.json.gz`;
    const bytes = gzipSync(JSON.stringify(events));
    files.set(`/run/replay/${file}`, bytes);
    chunks.push({ file, events: 32, compressed_bytes: bytes.length,
      sha256: createHash('sha256').update(bytes).digest('hex'), first_sequence: index * 32,
      first_at_ms: events[0].at_ms, last_at_ms: events.at(-1).at_ms });
  }
  const manifest = { version: 1, channel: 'run-progress', encoding: 'gzip-json-checkpoints',
    clock: 'elapsed-monotonic-milliseconds', complete: true, events: 128, duration_ms: 1275, chunks };
  files.set('/run/replay/manifest.json', Buffer.from(JSON.stringify(manifest)));
  const fetcher = async url => new Response(files.get(new URL(url, 'https://example.test/').pathname), { status: 200 });
  return { reader: new ReplayReader('replay/manifest.json', fetcher, 'https://example.test/run/index.html'), files, manifest };
}

test('virtual clock preserves position across pause, speed changes and seeking', () => {
  let wall = 0;
  const clock = new ReplayClock(10000, () => wall);
  clock.play();
  wall = 1000;
  assert.equal(clock.position(), 1000);
  clock.setSpeed(2);
  wall = 1500;
  assert.equal(clock.position(), 2000);
  clock.pause();
  wall = 9000;
  assert.equal(clock.position(), 2000);
  clock.seek(-10);
  assert.equal(clock.position(), 0);
  clock.seek(20000);
  assert.equal(clock.position(), 10000);
  assert.throws(() => clock.setSpeed(0));
});

test('seeking reconstructs exact checkpoints and keeps at most two decoded chunks', async () => {
  const { reader } = fixture();
  await reader.load();
  assert.equal(await reader.stateAt(0), null);
  for (const position of [5, 319, 325, 1275, 645, 5]) {
    const event = await reader.stateAt(position);
    assert.equal(event.state.completed, Math.floor((position - 5) / 10));
    assert.ok(reader.cache.size <= 2);
  }
});

test('manifest traversal, foreign origins and corrupt chunks are rejected', async () => {
  const { reader, manifest, files } = fixture();
  const invalid = structuredClone(manifest);
  invalid.chunks[0].file = '../private.json.gz';
  assert.throws(() => validateReplayManifest(invalid));
  assert.throws(() => new ReplayReader('https://other.test/replay/manifest.json', fetch, 'https://example.test/'));
  await reader.load();
  files.set('/run/replay/chunk-000000.json.gz', Buffer.from('corrupt'));
  await assert.rejects(reader.stateAt(5), /checksum/);
});

test('an aborted seek cannot populate the decoded cache', async () => {
  const { reader } = fixture();
  await reader.load();
  const abort = new AbortController();
  abort.abort();
  await assert.rejects(reader.stateAt(5, abort.signal), { name: 'AbortError' });
  assert.equal(reader.cache.size, 0);
});

test('gzip expansion is bounded before JSON parsing', async () => {
  const { reader, files, manifest } = fixture();
  const bytes = gzipSync(JSON.stringify([{ sequence: 0, at_ms: 0,
    state: { detail: 'x'.repeat(600 * 1024) } }]));
  manifest.events = 1;
  manifest.duration_ms = 0;
  manifest.chunks = [{ file: 'chunk-000000.json.gz', events: 1, first_sequence: 0,
    first_at_ms: 0, last_at_ms: 0, compressed_bytes: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex') }];
  files.set('/run/replay/manifest.json', Buffer.from(JSON.stringify(manifest)));
  files.set('/run/replay/chunk-000000.json.gz', bytes);
  await reader.load();
  await assert.rejects(reader.stateAt(0), /read budget/);
  assert.equal(reader.cache.size, 0);
});

test('cancellation after a response arrives still prevents cache publication', async () => {
  const { reader } = fixture();
  await reader.load();
  const abort = new AbortController();
  const original = reader.fetcher;
  reader.fetcher = async (...args) => {
    const response = await original(...args);
    abort.abort();
    return response;
  };
  await assert.rejects(reader.stateAt(5, abort.signal), { name: 'AbortError' });
  assert.equal(reader.cache.size, 0);
});

test('default fetch is called without the reader as its receiver', async () => {
  const { reader: fixtureReader } = fixture();
  const previous = globalThis.fetch;
  try {
    globalThis.fetch = function (...args) {
      assert.equal(this, undefined);
      assert.equal(typeof args[0], 'string');
      assert.match(args[0], /^\/run\/replay\//);
      return fixtureReader.fetcher(...args);
    };
    const reader = new ReplayReader('replay/manifest.json', undefined, 'https://example.test/run/index.html');
    await reader.load();
    assert.equal((await reader.stateAt(5)).sequence, 0);
  } finally {
    globalThis.fetch = previous;
  }
});
