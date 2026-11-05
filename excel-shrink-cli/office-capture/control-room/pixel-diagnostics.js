const cached = new Map();
const pending = new Map();
const queue = [];
let running = false;
const number = value => Number(value || 0).toLocaleString('en-US');

/** Descriptive measurements, never a severity threshold that certifies a pass. */
export function pixelSummary(metrics) {
  if (!metrics) return 'Pixel measurements pending';
  const text = `${number(metrics.changed_pixels)} changed px · max Δ${number(metrics.max_channel_delta)}/255 · ΣΔ ${number(metrics.total_channel_delta)}`;
  const exterior = metrics.unmatched_pixels
    ? ` · ${number(metrics.unmatched_pixels)} unpaired px (${number(metrics.unmatched_nonwhite_pixels)} non-white)` : '';
  const missing = metrics.unmeasured_regions ? ` · ${number(metrics.unmeasured_regions)} regions unavailable` : '';
  return text + exterior + missing;
}

export function cachedPixelDiagnostics(id) { return cached.get(id)?.value; }

/** Serialize image inspection; a selected atelier can move ahead of list work. */
export function requestPixelDiagnostics(id, { priority = false, refresh = false, region = null } = {}) {
  const key = region ? `${id}/${region}` : id;
  const previous = cached.get(key);
  if (!refresh && previous && Date.now() - previous.at < 60000) return Promise.resolve(previous.value);
  if (pending.has(key)) {
    const index = queue.findIndex(job => job.key === key);
    if (priority && index > 0) queue.unshift(queue.splice(index, 1)[0]);
    return pending.get(key);
  }
  const promise = new Promise((resolve, reject) => {
    const job = { id, key, region, resolve, reject };
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
      const value = await response.json();
      if (!response.ok) throw new Error(value.error || `HTTP ${response.status}`);
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
      element.textContent = 'Pixel measurements unavailable';
      element.title = error.message;
    }
  });
}
