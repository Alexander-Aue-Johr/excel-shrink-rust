import { captureFrames } from './capture-strip-model.js';

export const capturePalette = [
  { color: '#3498d3', opacity: 1, label: 'New' },
  { color: '#758a9a', opacity: .65, label: 'Previous' },
  { color: '#a6b2bc', opacity: .32, label: 'Earlier' },
];
const svgNS = 'http://www.w3.org/2000/svg';
const node = (tag, className, text) => {
  const result = document.createElement(tag);
  result.className = className;
  if (text != null) result.textContent = text;
  return result;
};
function svgNode(tag, attributes) {
  const result = document.createElementNS(svgNS, tag);
  for (const [key, value] of Object.entries(attributes)) result.setAttribute(key, String(value));
  return result;
}
const location = event => [event.details?.sheet_name, event.details?.range].filter(Boolean).join('!');

export function createCaptureSpatialView({ compact = false, onCaptureNavigate = null } = {}) {
  const root = node('details', 'capture-spatial');
  root.open = !compact;
  const summary = node('summary', 'capture-spatial-heading');
  const title = node('span', '', 'Worksheet capture area');
  const status = node('span', 'capture-spatial-status', 'Observed ranges');
  summary.append(title, status);
  const frames = node('div', 'capture-spatial-frames');
  const legend = node('div', 'capture-spatial-legend');
  for (const [generation, style] of capturePalette.entries()) {
    const item = node('span', `capture-generation capture-generation-${generation}`);
    const swatch = node('i', '');
    swatch.style.backgroundColor = style.color;
    swatch.style.opacity = style.opacity;
    item.append(swatch, document.createTextNode(style.label));
    legend.append(item);
  }
  const note = node('p', 'capture-spatial-note',
    'Up to three observed batches on the same worksheet. Times mark image receipt; select a tile to inspect its capture.');
  root.append(summary, frames, legend, note);
  let signature;

  function renderFrame(frame, index) {
    const card = node('div', 'capture-spatial-frame');
    card.dataset.frameId = frame.id;
    const at = new Date(frame.latest).toLocaleTimeString('en-US', { hour12: false });
    const label = node('div', 'capture-spatial-caption', `Batch ${index + 1} · ${at}`);
    const layers = [...(frame.layers || [])].sort((a, b) => b.generation - a.generation);
    if (!layers.some(layer => layer.rects?.length)) {
      card.append(node('div', 'capture-spatial-empty', `${frame.count} received · range unavailable`), label);
      return card;
    }
    const map = svgNode('svg', { class: 'capture-spatial-map', viewBox: '0 0 240 160',
      role: 'group', 'aria-label': `${frame.sheetName || 'Worksheet'} · batch received at ${at}` });
    map.append(svgNode('rect', { x: 1, y: 1, width: 238, height: 158, rx: 8, class: 'capture-spatial-board' }));
    const grid = svgNode('path', { d: 'M60 10V150 M120 10V150 M180 10V150 M10 40H230 M10 80H230 M10 120H230',
      class: 'capture-spatial-grid', 'aria-hidden': 'true' });
    map.append(grid);
    for (const layer of layers) {
      const style = capturePalette[layer.generation];
      if (!style) continue;
      for (const rect of layer.rects || []) {
        const event = rect.event;
        const tile = svgNode('rect', { class: 'capture-spatial-tile',
          x: 10 + rect.x * 220, y: 10 + rect.y * 140,
          width: Math.max(.6, rect.width * 220 - 2), height: Math.max(.6, rect.height * 140 - 2), rx: 3,
          fill: style.color, opacity: style.opacity, 'data-generation': layer.generation,
          'data-capture-range': event?.details?.range || '',
          'aria-label': `${style.label} capture · ${location(event || {}) || 'Worksheet range'}` });
        const tooltip = svgNode('title', {});
        tooltip.textContent = `${style.label} · ${location(event || {}) || 'Worksheet range'} · received ${new Date(event?.at ?? frame.latest).toLocaleTimeString('en-US')}`;
        tile.append(tooltip);
        if (onCaptureNavigate && event) {
          tile.setAttribute('role', 'button');
          tile.setAttribute('tabindex', '0');
          const select = input => {
            input.preventDefault(); input.stopPropagation();
            onCaptureNavigate({ capture: event, follow: false });
          };
          tile.addEventListener('click', select);
          tile.addEventListener('keydown', input => {
            if (input.key === 'Enter' || input.key === ' ') select(input);
          });
        }
        map.append(tile);
      }
    }
    card.append(map, label);
    if (frame.unlocatedCount) card.append(node('small', 'capture-spatial-note', `${frame.unlocatedCount} without a reported range`));
    return card;
  }

  return { element: root, update(events, now) {
    // The time lane expires after a minute. This bounded spatial context keeps
    // the last observed area visible while Excel is quiet or working elsewhere.
    const next = captureFrames(events, { now, windowMs: Number.MAX_SAFE_INTEGER, limit: 3 });
    const key = JSON.stringify(next);
    if (key === signature) return;
    signature = key;
    frames.replaceChildren(...next.map(renderFrame));
    if (!next.length) frames.append(node('p', 'capture-spatial-empty', 'No capture ranges observed yet.'));
    status.textContent = next.length ? `${next.at(-1).sheetName || 'Worksheet'} · ${next.length} generation${next.length === 1 ? '' : 's'}` : 'Waiting for captures';
    const visual = next.some(frame => frame.grouping !== 'command');
    note.textContent = next.length && visual
      ? 'Observed images grouped by receipt time; these are visual groups, not reported capture batches.'
      : 'Up to three observed batches on the same worksheet. Times mark image receipt; select a tile to inspect its capture.';
  } };
}
