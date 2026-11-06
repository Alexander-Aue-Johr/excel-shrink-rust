import { pixelSummary } from './pixel-summary.js';

const make = (tag, className, text) => {
  const element = document.createElement(tag);
  element.className = className;
  if (text != null) element.textContent = text;
  return element;
};
const svgNode = (tag, attributes) => {
  const element = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [key, value] of Object.entries(attributes)) element.setAttribute(key, value);
  return element;
};

/** Keep original images intact. All outlines, masks and enlarged crops are views
 * of retained evidence, never new Excel captures or revised comparison verdicts. */
export function createPixelInspector(views, metrics, region) {
  const root = make('section', 'atelier-pixel-inspector');
  const summary = make('p', 'atelier-pixel-summary', pixelSummary(metrics));
  const explanation = make('p', 'atelier-pixel-explanation',
    'Red: changed overlapping RGBA pixels. Amber: content without a matching capture pixel. '
    + 'ΣΔ is the sum of absolute channel differences, not a regression verdict. Click an outlined region or either image to inspect it.');
  if (!metrics.changed_pixels && region.geometry_differs) {
    explanation.textContent = 'No overlapping RGBA pixels changed. The comparison reports different capture geometry. '
      + 'Amber outlines locate unpaired content; this may be a cropped image edge rather than a workbook change.';
  }
  const toggle = make('button', '', 'Pause red pulse');
  toggle.type = 'button';
  toggle.addEventListener('click', () => {
    const paused = root.classList.toggle('pulse-paused');
    for (const view of views) view.stage.classList.toggle('pulse-paused', paused);
    toggle.textContent = paused ? 'Resume red pulse' : 'Pause red pulse';
  });
  const crops = make('div', 'atelier-pixel-crops');
  const more = make('button', '', 'Show more enlarged regions');
  more.type = 'button';
  const reset = make('button', '', 'Show detected regions');
  reset.type = 'button';
  reset.hidden = true;
  const coverage = make('p', 'atelier-pixel-explanation');
  root.append(summary, explanation, toggle, coverage, crops, more, reset);
  let mask = null;
  let visibleCount = 4;
  let focused = null;
  const hotspots = metrics.hotspots || [];

  function cropBox(spot) {
    const x = Math.max(0, spot.x - 8), y = Math.max(0, spot.y - 8);
    return {
      x, y,
      width: Math.max(1, Math.min(128, Math.max(40, spot.width + 16), Math.max(metrics.left_width, metrics.right_width) - x)),
      height: Math.max(1, Math.min(96, Math.max(40, spot.height + 16), Math.max(metrics.left_height, metrics.right_height) - y)),
    };
  }

  function paint(canvas, image, box, scale) {
    canvas.width = box.width * scale; canvas.height = box.height * scale;
    const context = canvas.getContext('2d');
    context.imageSmoothingEnabled = false;
    const width = Math.min(box.width, image.naturalWidth - box.x);
    const height = Math.min(box.height, image.naturalHeight - box.y);
    if (width > 0 && height > 0) context.drawImage(image,
      box.x, box.y, width, height, 0, 0, width * scale, height * scale);
  }

  function renderCrops() {
    crops.replaceChildren();
    const selected = focused ? [focused] : hotspots.slice(0, visibleCount);
    coverage.textContent = `${hotspots.length} of ${metrics.hotspot_count || 0} hotspot boxes retained. `
      + 'The red mask contains every changed overlapping pixel. Checkerboard means outside a retained capture.';
    for (const [index, spot] of selected.entries()) {
      const box = cropBox(spot);
      const scale = Math.max(1, Math.min(8, Math.floor(320 / box.width), Math.floor(220 / box.height)));
      const card = make('article', 'atelier-pixel-crop');
      card.append(make('h4', '', `${spot.unpaired ? 'Unpaired content' : 'Pixel region'} ${index + 1} · x ${spot.x}, y ${spot.y}`));
      const pair = make('div', 'atelier-crop-pair');
      for (const view of views) {
        const figure = make('figure', '');
        const stage = make('div', 'atelier-crop-stage');
        const canvas = make('canvas', '');
        paint(canvas, view.image, box, scale);
        stage.append(canvas);
        if (mask) {
          const overlay = make('canvas', 'atelier-diff-pulse');
          paint(overlay, mask, box, scale);
          stage.append(overlay);
        }
        figure.append(make('figcaption', '', view.label), stage);
        pair.append(figure);
      }
      card.append(pair); crops.append(card);
    }
    more.hidden = Boolean(focused) || visibleCount >= hotspots.length;
    reset.hidden = !focused;
    if (!selected.length) crops.append(make('p', '', 'No changed or unpaired non-white pixels to enlarge. Click an image for manual inspection.'));
  }

  function focus(spot) {
    focused = spot;
    renderCrops();
    const reducedMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;
    root.scrollIntoView({ block: 'nearest', behavior: reducedMotion ? 'instant' : 'smooth' });
  }
  reset.addEventListener('click', () => { focused = null; renderCrops(); });
  more.addEventListener('click', () => { visibleCount += 8; renderCrops(); });
  for (const view of views) {
    const svg = svgNode('svg', { viewBox: `0 0 ${view.image.naturalWidth} ${view.image.naturalHeight}`, class: 'atelier-pixel-overlay' });
    if (metrics.overlay_url) svg.append(svgNode('image', {
      href: metrics.overlay_url, x: 0, y: 0, width: metrics.overlap_width,
      height: metrics.overlap_height, class: 'atelier-diff-pulse',
    }));
    for (const spot of hotspots) {
      const rectangle = svgNode('rect', {
        x: Math.max(0, spot.x - 2), y: Math.max(0, spot.y - 2),
        width: spot.width + 4, height: spot.height + 4,
        class: spot.unpaired ? 'unpaired' : 'changed',
        'vector-effect': 'non-scaling-stroke', tabindex: 0, role: 'button',
        'aria-label': `${spot.unpaired ? 'Unpaired' : 'Changed'} pixels at ${spot.x}, ${spot.y}`,
      });
      rectangle.addEventListener('click', () => focus(spot));
      rectangle.addEventListener('keydown', event => { if (event.key === 'Enter') focus(spot); });
      svg.append(rectangle);
    }
    view.stage.append(svg);
    view.image.addEventListener('click', event => {
      const bounds = view.image.getBoundingClientRect();
      focus({ x: Math.floor((event.clientX - bounds.left) * view.image.naturalWidth / bounds.width),
        y: Math.floor((event.clientY - bounds.top) * view.image.naturalHeight / bounds.height), width: 1, height: 1 });
    });
  }
  if (metrics.overlay_url) {
    const image = new Image();
    image.onload = () => { mask = image; if (root.isConnected) renderCrops(); };
    image.src = metrics.overlay_url;
  }
  renderCrops();
  return root;
}
