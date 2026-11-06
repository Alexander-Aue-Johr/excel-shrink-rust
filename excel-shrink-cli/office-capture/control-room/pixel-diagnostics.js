import { readDiagnosticResponse } from './diagnostic-response.js';
import { DiagnosticCache } from './diagnostic-cache.js';
import { pixelSummary } from './pixel-summary.js';
export { pixelSummary } from './pixel-summary.js';

const cached = new DiagnosticCache({ maxEntries: 512, maxBytes: 8 * 1024 * 1024 });
const MAX_PENDING_REQUESTS = 128;
const pending = new Map();
const queue = [];
let running = false;

export function cachedPixelDiagnostics(id) { return cached.get(id)?.value; }

/** Serialize image inspection; a selected atelier can move ahead of list work. */
export function requestPixelDiagnostics(id, { priority = false, refresh = false, region = null } = {}) {
  const key = region ? `${id}/${region}` : id;
  const previous = cached.get(key);
  if (!refresh && previous && Date.now() - previous.at < 60000) return Promise.resolve(previous.value);
  if (pending.has(key)) {
    const index = queue.findIndex(job => job.key === key);
    if (priority && index >= 0) {
      queue[index].priority = true;
      if (index > 0) queue.unshift(queue.splice(index, 1)[0]);
    }
    return pending.get(key);
  }
  if (pending.size >= MAX_PENDING_REQUESTS) {
    // Do not let a long result list keep thousands of DOM rows and response
    // promises alive. An explicitly selected region may replace list prefetch.
    const disposable = priority ? queue.findLastIndex(job => !job.priority) : -1;
    if (disposable < 0) return Promise.reject(new Error('Pixel measurement queue is full; retry this view.'));
    const [deferred] = queue.splice(disposable, 1);
    pending.delete(deferred.key);
    deferred.reject(new Error('Pixel measurement deferred while inspecting a selected region.'));
  }
  const promise = new Promise((resolve, reject) => {
    const job = { id, key, region, priority, resolve, reject };
    if (priority) queue.unshift(job); else queue.push(job);
  });
  pending.set(key, promise);
  void pump();
  return promise;
}

async function pump() {
  if (running) return;
  running = true;
  while (queue.length) {
    const job = queue.shift();
    try {
      const query = job.region ? `?region=${encodeURIComponent(job.region)}` : '';
      const response = await fetch(`/api/groups/${encodeURIComponent(job.id)}/pixel-diagnostics${query}`, { cache: 'no-store' });
      const value = await readDiagnosticResponse(response, 'Pixel diagnostics');
      cached.set(job.key, { value, at: Date.now() });
      job.resolve(value);
    } catch (error) { job.reject(error); }
    finally { pending.delete(job.key); }
  }
  running = false;
}

/** Update only this row's summary; loading measurements never replaces the list. */
export function attachPixelSummary(element, id) {
  const show = value => {
    element.textContent = pixelSummary(value.summary);
    element.title = 'Exact RGBA differences in overlapping image pixels. ΣΔ sums absolute channel differences across compared regions. Unpaired pixels describe capture sizes, not changed shared pixels.';
  };
  const previous = cachedPixelDiagnostics(id);
  if (previous) show(previous); else element.textContent = 'Measuring retained pixels…';
  requestPixelDiagnostics(id).then(value => {
    if (element.isConnected) show(value);
  }).catch(error => {
    if (element.isConnected) {
      element.textContent = error.code === 'unsupported-diagnostic-endpoint'
        ? 'Pixel measurements require the current Observatory server'
        : 'Pixel measurements unavailable';
      element.title = error.message;
    }
  });
}
