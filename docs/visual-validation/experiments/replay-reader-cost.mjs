/** Read an existing synthetic recording through the production reader over static HTTP. */
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { readFile, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import path from 'node:path';
import { ReplayReader } from '../../../excel-shrink-cli/office-capture/control-room/replay-engine.js';

const [directory, receiptPath] = process.argv.slice(2);
assert.ok(directory && receiptPath, 'Supply the existing recording directory and a new receipt path.');
assert.equal(typeof global.gc, 'function', 'Run with --expose-gc for labelled post-GC samples.');
const root = path.resolve(directory);
const sourceManifest = await readFile(path.join(root, 'manifest.json'));
const server = createServer((request, response) => {
  const match = /^\/replay\/(manifest\.json|chunk-\d{6}\.json\.gz)$/.exec(request.url);
  if (!match) {
    response.writeHead(404).end();
    return;
  }
  const stream = createReadStream(path.join(root, match[1]));
  stream.on('error', () => response.writeHead(404).end());
  response.setHeader('Content-Type', match[1].endsWith('.gz') ? 'application/gzip' : 'application/json');
  stream.pipe(response);
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));

let requests = 0;
let maximumCacheChunks = 0;
let maximumCachedEvents = 0;
const memory = [];
const started = performance.now();
function sample(label) {
  global.gc();
  memory.push({ label, ...process.memoryUsage() });
}

try {
  const base = `http://127.0.0.1:${server.address().port}/`;
  const reader = new ReplayReader('replay/manifest.json', (relative, options) => {
    requests += 1;
    return fetch(new URL(relative, base), options);
  }, `${base}index.html`);
  const manifest = await reader.load();
  assert.equal(manifest.events, 100000);
  assert.equal(await reader.stateAt(-1), null);
  sample('manifest loaded');
  for (let sequence = 0; sequence < manifest.events; sequence += 1) {
    const event = await reader.stateAt(sequence * 1000 + 999);
    assert.equal(event.sequence, sequence);
    assert.equal(event.state.completed, Math.floor(sequence / 5));
    maximumCacheChunks = Math.max(maximumCacheChunks, reader.cache.size);
    maximumCachedEvents = Math.max(maximumCachedEvents,
      [...reader.cache.values()].reduce((sum, events) => sum + events.length, 0));
    assert.ok(reader.cache.size <= 2);
    if ((sequence + 1) % 10000 === 0) sample(`after ${sequence + 1} checkpoints`);
  }
  for (let index = 0; index < 128; index += 1) {
    const sequence = (index * 7919) % manifest.events;
    const event = await reader.stateAt(sequence * 1000);
    assert.equal(event.sequence, sequence);
    assert.ok(reader.cache.size <= 2);
  }
  sample('after 128 nonsequential seeks');
  const receipt = {
    schema: 1,
    synthetic: true,
    engine: 'production ReplayReader in Node.js over loopback static HTTP',
    node_version: process.version,
    manifest_sha256: createHash('sha256').update(sourceManifest).digest('hex'),
    events_verified: manifest.events,
    nonsequential_seeks: 128,
    requests,
    maximum_cache_chunks: maximumCacheChunks,
    maximum_cached_events: maximumCachedEvents,
    elapsed_seconds: (performance.now() - started) / 1000,
    post_gc_memory_samples: memory,
    limitations: [
      'Node process includes the static server, HTTP client, test loop and reader.',
      'Post-GC samples are observations and do not establish transient allocation peaks.',
      'Manifest metadata remains resident and grows with the number of chunks.',
      'This exercises the reader and HTTP transport, excluding browser UI, DOM, CSP and rendering.',
    ],
  };
  await writeFile(receiptPath, JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx' });
  console.log(JSON.stringify(receipt));
} finally {
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
}
