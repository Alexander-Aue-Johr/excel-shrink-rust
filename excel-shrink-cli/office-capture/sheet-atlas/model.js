// Static report transport and retained capture coordinates.
export const atlasRoot = new URL('../', import.meta.url);
const details = new Map();
const images = new Map();

export function assetUrl(relative) {
  const url = new URL(relative, atlasRoot);
  if (url.origin !== atlasRoot.origin || !url.pathname.startsWith(atlasRoot.pathname)) {
    throw new Error('Atlas asset escapes the report directory.');
  }
  return url.href;
}

export async function readGzip(relative) {
  const response = await fetch(assetUrl(relative), { redirect: 'error', cache: 'no-cache' });
  if (!response.ok) throw new Error(`Report request failed (${response.status}).`);
  const chunks = [];
  let length = 0;
  const reader = response.body.getReader();
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    length += value.length;
    if (length > 128 * 1024 * 1024 + 65536) { await reader.cancel(); throw new Error('Compressed report exceeds its 128 MiB bound.'); }
    chunks.push(value);
  }
  const bytes = new Uint8Array(length);
  let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  // A host may already have decoded Content-Encoding. Inspect the actual bytes.
  const gzip = bytes[0] === 0x1f && bytes[1] === 0x8b;
  const stream = new Blob([bytes]).stream();
  const decoded = gzip ? stream.pipeThrough(new DecompressionStream('gzip')) : stream;
  const output = decoded.getReader();
  const text = [];
  const decoder = new TextDecoder();
  let expanded = 0;
  for (;;) {
    const { value, done } = await output.read();
    if (done) break;
    expanded += value.length;
    if (expanded > 128 * 1024 * 1024) { await output.cancel(); throw new Error('Expanded report exceeds 128 MiB.'); }
    text.push(decoder.decode(value, { stream: true }));
  }
  text.push(decoder.decode());
  return JSON.parse(text.join(''));
}

export function changed(finding) {
  return ['pixels_differ', 'geometry_differs', 'presence_differs', 'rendering_differs'].some(key => finding[key] === true);
}

export function recordedOutcome(finding) {
  if (changed(finding)) return 'different';
  return finding.pixels_compared === true ? 'equal' : 'unknown';
}

export function groupOutcome(variants, outcomes) {
  const values = variants.map(id => outcomes[id] || 'unknown');
  if (values.includes('different')) return 'different';
  return values.every(value => value === 'equal') ? 'equal' : 'unknown';
}

export function candidatePair(pair) {
  return ['O', 'OR'].includes(pair.left) && ['SD', 'SDR'].includes(pair.right);
}

export function sheetSummary(observation, name) {
  return observation.sheets && Object.hasOwn(observation.sheets, name) ? observation.sheets[name] : undefined;
}

export function sheetEvidence(summaries) {
  const differences = summaries.reduce((sum, value) => sum + (value.differences || 0), 0);
  const captures = summaries.reduce((sum, value) => sum + (value.captures || 0), 0);
  const planned = summaries.some(value => value.planning?.selected_capture_regions > 0);
  const rules = summaries.flatMap(value => value.conditional_formatting || []);
  const indirect = rules.some(rule => ['partial', 'capture'].includes(rule.outcome)
    && (rule.referenced_input_changes > 0 || rule.formula_input_observations > 0));
  const conservative = rules.some(rule => rule.outcome === 'capture'
    && rule.reason === 'unsupported-rule-semantics');
  const outcome = differences ? 'difference' : captures ? 'checked' : 'unknown';
  const labels = {
    difference: 'Δ Recorded difference · cause unclassified',
    checked: '= No difference in compared captures',
    unknown: '? No pixel comparison retained',
  };
  return { outcome, label: labels[outcome], planned, indirect, conservative, captures, differences };
}

export function loadDetail(observation) {
  if (!observation.detail) return Promise.resolve(null);
  if (!details.has(observation.detail)) {
    const pending = readGzip(observation.detail).catch(error => { details.delete(observation.detail); throw error; });
    details.set(observation.detail, pending);
    if (details.size > 12) details.delete(details.keys().next().value);
  }
  return details.get(observation.detail);
}

export function loadImage(url) {
  if (!images.has(url)) {
    const pending = new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => image.naturalWidth * image.naturalHeight <= 32_000_000
        ? resolve(image) : reject(new Error('Capture exceeds the 32 megapixel inspection limit.'));
      image.onerror = () => reject(new Error('Retained capture could not be loaded.'));
      image.src = url;
    }).catch(error => { images.delete(url); throw error; });
    images.set(url, pending);
    if (images.size > 24) images.delete(images.keys().next().value);
  }
  return images.get(url);
}

function geometry(finding, side) {
  const g = finding[`${side}_geometry`] || {};
  const scale = 4 / 3;
  return { x: Number(g.left || 0) * scale, y: Number(g.top || 0) * scale,
    w: Math.max(1, Number(g.width || 1) * scale), h: Math.max(1, Number(g.height || 1) * scale) };
}

export async function loadSheet(book, sheet, selected) {
  const result = { ...sheet, id: `${book.id}:${sheet.id}`, workbookId: book.id,
    width: 1, height: 1, changes: [], captures: [], kind: 'normal' };
  const observations = book.observations.filter(o => selected.has(o.variant));
  for (const observation of observations) {
    const payload = await loadDetail(observation);
    if (!payload) continue;
    for (const pair of payload.report.comparisons || []) {
      if (!candidatePair(pair)) continue;
      for (const [kind, findings] of [['findings', pair.findings], ['charts', pair.charts]]) {
        for (const [index, finding] of (findings || []).entries()) {
          if (finding.sheet !== sheet.name) continue;
          const bounds = geometry(finding, 'left');
          const left = finding.left_png || finding.left_image;
          const right = finding.right_png || finding.right_image;
          const capture = { ...bounds, id: `${observation.id}:${pair.left}:${pair.right}:${kind}:${index}`,
            variant: observation.variant, range: finding.cell || finding.range || finding.chart || 'Chart',
            state: `${pair.left} → ${pair.right}`, finding,
            original: left ? assetUrl(payload.image_base + left) : null,
            candidate: right ? assetUrl(payload.image_base + right) : null,
            page: assetUrl(observation.page), signature: payload.validation_signature };
          result.captures.push(capture);
          result.width = Math.max(result.width, bounds.x + bounds.w);
          result.height = Math.max(result.height, bounds.y + bounds.h);
          if (changed(finding)) result.changes.push({ ...capture, label: `${capture.range} · ${capture.state}` });
        }
      }
    }
  }
  const ratio = result.width / result.height;
  result.kind = ratio > 4 ? 'wide' : ratio < .5 ? 'tall' : result.width > 3500 && result.height > 3500 ? 'square' : 'normal';
  const first = result.captures.find(capture => capture.original);
  result.previewRegion = first ? { x: first.x, y: first.y, w: first.w, h: first.h } : { x: 0, y: 0, w: 1, h: 1 };
  if (first) result.previewImage = await loadImage(first.original);
  return result;
}

// Only an identical retained original image, range, native state and geometry
// authorize showing another processor in the same comparison context.
export function contextKey(capture) {
  return JSON.stringify([capture.range, capture.state, capture.original?.split('/').pop(),
    capture.finding.left_geometry, capture.signature]);
}

export async function comparisonContext(sheet, change, names) {
  const matching = sheet.captures.filter(capture => contextKey(capture) === contextKey(change));
  const captures = {};
  const notes = { original: 'Native reference capture' };
  const outcomes = { original: 'equal' };
  if (change.original) captures.original = await loadImage(change.original);
  for (const capture of matching) {
    if (capture.candidate) captures[capture.variant] = await loadImage(capture.candidate);
    const finding = capture.finding;
    outcomes[capture.variant] = recordedOutcome(finding);
    notes[capture.variant] = [
      finding.pixels_differ && 'Pixel difference recorded',
      finding.geometry_differs && 'Sheet geometry differs',
      finding.presence_differs && 'Presence differs',
      finding.rendering_differs && 'Rendering differs',
      finding.capture_extent_ignored && 'Blank capture extent ignored by validator'
    ].filter(Boolean).join(' · ') || 'No difference recorded for this context';
  }
  const widths = Object.values(captures).map(image => image.naturalWidth);
  const heights = Object.values(captures).map(image => image.naturalHeight);
  const width = Math.max(1, ...widths), height = Math.max(1, ...heights);
  if (width * height * Object.keys(captures).length > 32_000_000) {
    throw new Error('Native comparison canvases exceed the 32 megapixel inspection limit. Open the detailed source report.');
  }
  return { sheet: { ...sheet, width, height, nativeCaptures: captures },
    change: { ...change, x: 0, y: 0, w: width, h: height, native: true },
    variants: Object.keys(captures), names, notes, outcomes, changes: [], motion: true };
}
