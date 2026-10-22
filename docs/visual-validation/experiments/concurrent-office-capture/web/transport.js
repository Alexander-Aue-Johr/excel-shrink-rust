(function (root) {
  'use strict';
  let config, request;
  const metrics = {office_ms: 0, capture_ms: 0, hash_ms: 0, offer_wait_ms: 0,
    upload_wait_ms: 0, image_bytes: 0, batches: 0, max_batch: 0, images: 0};
  function record(name, value) { metrics[name] = (metrics[name] || 0) + value; }
  function batchEnd(tiles, offset, count, budget) {
    let end = offset, pixels = 0;
    while (end < tiles.length && end - offset < count) {
      const size = tiles[end].width * tiles[end].expectedHeight;
      if (size > budget) throw new Error('One tile exceeds the batch pixel budget');
      if (pixels + size > budget) break;
      pixels += size; end++;
    }
    return end;
  }
  async function bytes(base64) {
    const binary = atob(base64), result = new Uint8Array(binary.length);
    for (let start = 0; start < binary.length; start += 1048576) {
      const end = Math.min(start + 1048576, binary.length);
      for (let i = start; i < end; i++) result[i] = binary.charCodeAt(i);
      if (start) await new Promise(resolve => setTimeout(resolve, 0));
    }
    return result;
  }
  async function upload(metadata, base64) {
    const png = await bytes(base64);
    metadata.width = CaptureGeometry.pngSize(base64).width;
    metadata.height = CaptureGeometry.pngSize(base64).height;
    if (config.transport === 'hash') {
      const before = performance.now();
      metadata.sha256 = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', png)),
        b => b.toString(16).padStart(2, '0')).join('');
      record('hash_ms', performance.now() - before);
    }
    let before = performance.now();
    const decision = await request('/offer', metadata);
    record('offer_wait_ms', performance.now() - before);
    if (decision.upload) {
      before = performance.now();
      const response = await fetch('/image/' + encodeURIComponent(config.run_id) + '/' +
        metadata.sheet_index + '/' + metadata.tile_index, {
        method: 'POST', headers: {'Content-Type': 'image/png'}, body: png,
        signal: AbortSignal.timeout(120000)
      });
      if (!response.ok) throw new Error('Binary upload: ' + response.status + ' ' + await response.text());
      await response.json();
      record('upload_wait_ms', performance.now() - before);
      record('image_bytes', png.length);
    }
    record('images', 1);
  }
  const api = {metrics, record, batchEnd, upload,
    configure(value, send) { config = value; request = send; },
    begin() { for (const name of Object.keys(metrics)) metrics[name] = 0; },
    batch(count) { record('batches', 1); metrics.max_batch = Math.max(metrics.max_batch, count); }};
  root.Experiment = api;
  if (typeof module !== 'undefined') module.exports = api;
}(typeof window === 'undefined' ? globalThis : window));
