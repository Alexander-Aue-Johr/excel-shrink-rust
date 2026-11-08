/** A virtual clock whose reads never change playback state. */
export class ReplayClock {
  constructor(duration, now = () => performance.now()) {
    if (!Number.isSafeInteger(duration) || duration < 0) throw new Error('Invalid replay duration.');
    this.duration = duration;
    this.now = now;
    this.anchor = 0;
    this.wall = now();
    this.speed = 1;
    this.playing = false;
  }

  position() {
    const elapsed = this.playing ? (this.now() - this.wall) * this.speed : 0;
    return Math.min(this.duration, Math.max(0, this.anchor + elapsed));
  }

  seek(position) {
    if (!Number.isFinite(position)) throw new Error('Invalid replay position.');
    this.anchor = Math.min(this.duration, Math.max(0, position));
    this.wall = this.now();
  }

  play() {
    if (this.position() >= this.duration) this.seek(0);
    else this.seek(this.position());
    this.playing = true;
  }

  pause() {
    this.seek(this.position());
    this.playing = false;
  }

  setSpeed(speed) {
    if (!Number.isFinite(speed) || speed <= 0 || speed > 10000) throw new Error('Invalid replay speed.');
    this.seek(this.position());
    this.speed = speed;
  }
}

function replayInteger(value, label) {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`Invalid replay ${label}.`);
  return value;
}

async function readReplayBytes(stream, limit, signal) {
  const reader = stream.getReader();
  const parts = [];
  let length = 0;
  try {
    while (true) {
      signal?.throwIfAborted();
      const { value, done } = await reader.read();
      signal?.throwIfAborted();
      if (done) break;
      length += value.byteLength;
      if (length > limit) throw new Error('Replay data exceeds its read budget.');
      parts.push(value);
    }
  } catch (error) {
    await reader.cancel().catch(() => {});
    throw error;
  } finally {
    reader.releaseLock();
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) { bytes.set(part, offset); offset += part.byteLength; }
  return bytes;
}

/** Validate the fixed format before its index can select a file or allocate a cache. */
export function validateReplayManifest(manifest) {
  if (manifest?.version !== 1 || manifest.channel !== 'run-progress'
      || manifest.encoding !== 'gzip-json-checkpoints'
      || manifest.clock !== 'elapsed-monotonic-milliseconds' || manifest.complete !== true
      || !Array.isArray(manifest.chunks)) throw new Error('Replay history is unavailable or unsupported.');
  let sequence = 0;
  let lastAt = 0;
  manifest.chunks.forEach((chunk, index) => {
    const name = `chunk-${String(index).padStart(6, '0')}.json.gz`;
    const count = replayInteger(chunk.events, 'chunk count');
    const first = replayInteger(chunk.first_at_ms, 'chunk start');
    const last = replayInteger(chunk.last_at_ms, 'chunk end');
    const bytes = replayInteger(chunk.compressed_bytes, 'chunk size');
    if (chunk.file !== name || chunk.first_sequence !== sequence || count < 1 || count > 32
        || first < lastAt || last < first || bytes > 513 * 1024
        || !/^[a-f0-9]{64}$/.test(chunk.sha256)) throw new Error('Invalid replay chunk index.');
    sequence = replayInteger(sequence + count, 'event count');
    lastAt = last;
  });
  if (manifest.events !== sequence || manifest.duration_ms !== lastAt) throw new Error('Replay totals differ.');
  return manifest;
}

/** Fetch same-origin chunks on demand, retaining at most two decoded arrays. */
export class ReplayReader {
  constructor(manifestUrl, fetcher = (url, options) => fetch(url, options), base = location.href) {
    const page = new URL(base);
    this.url = new URL(manifestUrl, page);
    if (!/^https?:$/.test(page.protocol) || this.url.origin !== page.origin
        || !this.url.pathname.endsWith('/replay/manifest.json') || this.url.search || this.url.hash) {
      throw new Error('Replay requires a same-origin HTTP or HTTPS static site.');
    }
    this.fetcher = fetcher;
    this.cache = new Map();
    this.manifest = null;
  }

  async fetchBytes(url, limit, signal) {
    const response = await this.fetcher(url.pathname, { signal, redirect: 'error', credentials: 'same-origin' });
    if (!response.ok || !response.body) throw new Error(`Replay request failed (${response.status}).`);
    return readReplayBytes(response.body, limit, signal);
  }

  async load(signal) {
    const bytes = await this.fetchBytes(this.url, 8 * 1024 * 1024, signal);
    this.manifest = validateReplayManifest(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes)));
    this.cache.clear();
    return this.manifest;
  }

  async chunk(index, signal) {
    signal?.throwIfAborted();
    if (this.cache.has(index)) {
      const value = this.cache.get(index);
      this.cache.delete(index);
      this.cache.set(index, value);
      return value;
    }
    const entry = this.manifest.chunks[index];
    const bytes = await this.fetchBytes(new URL(entry.file, this.url), 513 * 1024, signal);
    const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', bytes));
    const digest = Array.from(hash, value => value.toString(16).padStart(2, '0')).join('');
    if (bytes.byteLength !== entry.compressed_bytes || digest !== entry.sha256) throw new Error('Replay checksum differs.');
    const decoded = await readReplayBytes(new Blob([bytes]).stream().pipeThrough(new DecompressionStream('gzip')),
      512 * 1024 + 64, signal);
    const events = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(decoded));
    if (!Array.isArray(events) || events.length !== entry.events) throw new Error('Replay event count differs.');
    let at = entry.first_at_ms;
    events.forEach((event, position) => {
      const nextAt = replayInteger(event.at_ms, 'timestamp');
      if (event.sequence !== entry.first_sequence + position || nextAt < at
          || !event.state || typeof event.state !== 'object' || Array.isArray(event.state)
          || new TextEncoder().encode(JSON.stringify(event)).length > 64 * 1024) {
        throw new Error('Replay event order or state is invalid.');
      }
      at = nextAt;
    });
    if (events[0].at_ms !== entry.first_at_ms || at !== entry.last_at_ms) throw new Error('Replay timestamp bounds differ.');
    signal?.throwIfAborted();
    this.cache.set(index, events);
    if (this.cache.size > 2) this.cache.delete(this.cache.keys().next().value);
    return events;
  }

  async stateAt(position, signal) {
    signal?.throwIfAborted();
    if (!this.manifest || !Number.isFinite(position)) throw new Error('Replay is not ready.');
    const entries = this.manifest.chunks;
    let low = 0;
    let high = entries.length;
    while (low < high) {
      const middle = Math.floor((low + high) / 2);
      if (entries[middle].first_at_ms <= position) low = middle + 1;
      else high = middle;
    }
    if (low === 0) return null;
    const events = await this.chunk(low - 1, signal);
    let selected = null;
    for (const event of events) {
      if (event.at_ms > position) break;
      selected = event;
    }
    return selected;
  }
}
