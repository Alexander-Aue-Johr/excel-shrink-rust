import { drawSheetRegion } from './sheet-renderer.js';
import { formatRange } from './data.js';
import { groupOutcome } from './model.js';

const NAMES = {
  original: 'Original',
};

const CSS = `
.scv-host{height:100%;min-height:0;min-width:0;overflow:auto}
.scv{--scv-panel:var(--panel,#15242c);--scv-text:var(--text,#e8f0ed);--scv-muted:var(--muted,#9db0b5);--scv-line:var(--line,#344a54);height:100%;min-height:300px;min-width:0;display:flex;flex-direction:column;gap:12px;color:var(--scv-text);font-family:Inter,'Segoe UI',Arial,sans-serif;box-sizing:border-box}
.scv *{box-sizing:border-box}.scv button,.scv input{font:inherit}.scv button{cursor:pointer;color:inherit}
.scv-toolbar{display:flex;flex-wrap:wrap;align-items:center;gap:12px 20px;flex:none;min-height:36px}
.scv-switch{display:inline-flex;gap:3px;padding:3px;border:1px solid var(--scv-line);border-radius:7px;background:var(--scv-panel)}
.scv-switch button{border:0;border-radius:4px;background:transparent;padding:7px 11px;color:var(--scv-muted);font-size:12px;white-space:nowrap}
.scv-switch button[aria-pressed=true]{background:#36534e;color:#f0f7f3}
.scv-check{display:flex;align-items:center;gap:7px;cursor:pointer;font-size:12px;color:var(--scv-muted);white-space:nowrap}
.scv-check input{width:14px;height:14px;margin:0;accent-color:#98c8b6}.scv-check:has(input:disabled){opacity:.55;cursor:default}
.scv button:focus-visible,.scv input:focus-visible,.scv:focus-visible{outline:2px solid #b3d7c8;outline-offset:3px}
.scv-nav{display:flex;align-items:center;gap:10px;margin-left:auto;white-space:nowrap}
.scv-nav button{border:1px solid var(--scv-line);border-radius:5px;background:transparent;width:31px;height:31px;font-size:17px}
.scv-nav button:disabled{opacity:.35;cursor:default}.scv-position{min-width:38px;text-align:center;color:var(--scv-muted);font-size:12px;font-variant-numeric:tabular-nums}
.scv-grid{flex:1;min-height:0;min-width:0;display:grid;gap:12px;overflow:auto;align-content:stretch;padding:1px}
.scv-pane{min-height:200px;min-width:0;display:flex;flex-direction:column;background:var(--scv-panel);border:1px solid var(--scv-line);border-radius:9px;overflow:hidden}
.scv-pane-head{height:66px;min-height:66px;display:flex;flex-direction:column;justify-content:center;padding:7px 12px;gap:3px;overflow:hidden;border-bottom:1px solid var(--scv-line)}
.scv-pane-head[data-outcome=equal]{background:#234d40;color:#e0f5e8;border-color:#638f7e}
.scv-pane-head[data-outcome=different]{background:#5a3039;color:#ffe5e8;border-color:#a4646d}
.scv-pane-head[data-outcome=unknown]{background:#293e48;color:#d9e3e7}
.scv-names{font-size:12px;font-weight:500;line-height:17px;overflow-wrap:anywhere;display:-webkit-box;-webkit-line-clamp:2;-webkit-box-orient:vertical;overflow:hidden}
.scv-pane-note{font-size:10px;line-height:13px;color:var(--scv-muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.scv-stage{position:relative;min-height:130px;flex:1;background:#eef2ef;overflow:hidden}
.scv-stage canvas{position:absolute;inset:0;width:100%;height:100%;display:block}.scv-image{cursor:crosshair}.scv-emphasis{pointer-events:none}
.scv.scv-motion .scv-emphasis{animation:scv-contour-pulse 1800ms ease-in-out infinite}
.scv-footer{display:flex;justify-content:space-between;align-items:center;gap:8px 18px;flex-wrap:wrap;color:var(--scv-muted);font-size:11px;line-height:1.5;flex:none}
.scv-footer span{min-width:0}.scv-empty{display:flex;align-items:center;justify-content:center;flex:1;min-height:180px;color:var(--scv-muted);font-size:13px}
@keyframes scv-contour-pulse{0%,100%{opacity:1}50%{opacity:.38}}
@media(prefers-reduced-motion:reduce){.scv.scv-motion .scv-emphasis{animation:none}}
@media(max-width:560px){.scv{gap:9px}.scv-toolbar{gap:9px 12px}.scv-switch button{font-size:11px;padding:6px 9px}.scv-check{font-size:11px}.scv-nav{gap:6px}.scv-grid{gap:9px}.scv-pane-head{padding:6px 9px}.scv-footer{font-size:10px}}
`;

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function button(text, label, handler) {
  const node = el('button', '', text);
  node.type = 'button';
  node.setAttribute('aria-label', label);
  node.title = label;
  node.addEventListener('click', handler);
  return node;
}

function nativeCanvas(width, height) {
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  return canvas;
}

function samePixels(a, b) {
  if (a.length !== b.length) return false;
  const aa = new Uint32Array(a.buffer, a.byteOffset, a.byteLength / 4);
  const bb = new Uint32Array(b.buffer, b.byteOffset, b.byteLength / 4);
  for (let i = 0; i < aa.length; i++) if (aa[i] !== bb[i]) return false;
  return true;
}

function difference(a, b) {
  const mask = new Uint8Array(a.length / 4);
  const aa = new Uint32Array(a.buffer, a.byteOffset, a.byteLength / 4);
  const bb = new Uint32Array(b.buffer, b.byteOffset, b.byteLength / 4);
  for (let i = 0; i < mask.length; i++) mask[i] = aa[i] !== bb[i] ? 1 : 0;
  return mask;
}

function contextFor(sheet, change) {
  if (change.native) return { x: 0, y: 0, w: sheet.width, h: sheet.height };
  const w = Math.max(1, Math.min(Math.floor(sheet.width), Math.min(1200, Math.max(900, Math.ceil(change.w) + 160))));
  const h = Math.max(1, Math.min(Math.floor(sheet.height), Math.min(720, Math.max(450, Math.ceil(change.h) + 140))));
  const x = Math.round(Math.max(0, Math.min(sheet.width - w, change.x + change.w / 2 - w / 2)));
  const y = Math.round(Math.max(0, Math.min(sheet.height - h, change.y + change.h / 2 - h / 2)));
  return { x, y, w, h };
}

function fitImage(source, box) {
  const scale = Math.max(.001, Math.min(box.w / source.w, box.h / source.h));
  const w = source.w * scale, h = source.h * scale;
  return { x: box.x + (box.w - w) / 2, y: box.y + (box.h - h) / 2, w, h, scale };
}

function intersect(a, b) {
  const x = Math.max(a.x, b.x), y = Math.max(a.y, b.y);
  return { x, y, w: Math.max(0, Math.min(a.x + a.w, b.x + b.w) - x), h: Math.max(0, Math.min(a.y + a.h, b.y + b.h) - y) };
}

function maskImage(pixels, mask, width, height) {
  const canvas = nativeCanvas(width, height);
  const ctx = canvas.getContext('2d');
  const image = ctx.createImageData(width, height);
  for (let i = 0; i < mask.length; i++) {
    const p = i * 4;
    if (mask[i]) {
      image.data[p] = 183; image.data[p + 1] = 58; image.data[p + 2] = 53;
    } else {
      const grey = Math.round(178 + (pixels[p] * .2126 + pixels[p + 1] * .7152 + pixels[p + 2] * .0722) * .29);
      image.data[p] = grey; image.data[p + 1] = Math.min(255, grey + 3); image.data[p + 2] = grey;
    }
    image.data[p + 3] = 255;
  }
  ctx.putImageData(image, 0, 0);
  return canvas;
}

function contourImage(mask, width, height) {
  // A two-source-pixel outer contour leaves the evidence pixels untouched.
  const spread = new Uint8Array(mask.length);
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const i = y * width + x;
      if (!mask[i]) continue;
      for (let dy = -2; dy <= 2; dy++) for (let dx = -2; dx <= 2; dx++) {
        const xx = x + dx, yy = y + dy;
        if (xx >= 0 && yy >= 0 && xx < width && yy < height) spread[yy * width + xx] = 1;
      }
    }
  }
  const canvas = nativeCanvas(width, height);
  const ctx = canvas.getContext('2d');
  const image = ctx.createImageData(width, height);
  for (let i = 0; i < mask.length; i++) {
    if (!spread[i] || mask[i]) continue;
    const p = i * 4;
    image.data[p] = 192; image.data[p + 1] = 58; image.data[p + 2] = 54; image.data[p + 3] = 215;
  }
  ctx.putImageData(image, 0, 0);
  return canvas;
}

/**
 * Pixel-faithful comparison of exactly the variants supplied by the caller.
 * Equality is evaluated on native RGBA pixels of one shared context crop.
 */
export function nativeZoomRegion(mask, width, height) {
  const first = mask.indexOf(1);
  const cx = first < 0 ? width / 2 : first % width;
  const cy = first < 0 ? height / 2 : Math.floor(first / width);
  const w = Math.min(240, width), h = Math.min(120, height);
  return { x: Math.round(Math.max(0, Math.min(width - w, cx - w / 2))),
    y: Math.round(Math.max(0, Math.min(height - h, cy - h / 2))), w, h };
}

export class ComparisonView {
  constructor(container, { onBack, onSelectChange } = {}) {
    this.container = container;
    this.callbacks = { onBack, onSelectChange };
    this.grouped = true;
    this.maskMode = false;
    this.pulse = true;
    this.motion = true;
    this.frame = 0;
    this.panes = [];
    this.images = new Map();
    this.destroyed = false;
    this.hadHostClass = container.classList.contains('scv-host');
    container.classList.add('scv-host');
    if (!document.getElementById('sheet-atlas-comparison-style')) {
      const style = el('style');
      style.id = 'sheet-atlas-comparison-style';
      style.textContent = CSS;
      document.head.append(style);
    }
    this.root = el('section', 'scv');
    this.root.setAttribute('aria-label', 'Pixel comparison of selected variants');
    this.root.tabIndex = 0;
    container.append(this.root);
    this._buildControls();
    this.media = window.matchMedia('(prefers-reduced-motion: reduce)');
    this.onMediaChange = () => this._motionChanged();
    this.media.addEventListener?.('change', this.onMediaChange);
    this.observer = new ResizeObserver(() => this._schedule());
    this.observer.observe(container);
    this.root.addEventListener('keydown', event => {
      if (['INPUT', 'SELECT', 'TEXTAREA'].includes(event.target.tagName)) return;
      if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') {
        event.preventDefault(); event.stopPropagation();
        this._step(event.key === 'ArrowLeft' ? -1 : 1);
      } else if (event.key === 'Escape' && this.callbacks.onBack) {
        event.preventDefault(); event.stopPropagation(); this.callbacks.onBack();
      }
    });
    this._motionChanged();
  }

  _buildControls() {
    this.toolbar = el('div', 'scv-toolbar');
    const modes = el('div', 'scv-switch');
    modes.setAttribute('role', 'group');
    modes.setAttribute('aria-label', 'Image display');
    this.imagesButton = button('Original images', 'Show unmodified images', () => this._setMask(false));
    this.maskButton = button('Difference mask', 'Highlight changed original pixels', () => this._setMask(true));
    this.imagesButton.setAttribute('aria-pressed', 'true');
    this.maskButton.setAttribute('aria-pressed', 'false');
    modes.append(this.imagesButton, this.maskButton);
    const groupLabel = el('label', 'scv-check');
    this.groupInput = el('input'); this.groupInput.type = 'checkbox'; this.groupInput.checked = true;
    this.groupInput.addEventListener('change', () => { this.grouped = this.groupInput.checked; this._buildPanes(); });
    groupLabel.append(this.groupInput, el('span', '', 'Group identical images'));
    groupLabel.title = 'Group exactly pixel-identical images in the displayed context';
    const pulseLabel = el('label', 'scv-check');
    this.pulseInput = el('input'); this.pulseInput.type = 'checkbox'; this.pulseInput.checked = true;
    this.pulseInput.addEventListener('change', () => { this.pulse = this.pulseInput.checked; this._motionChanged(); });
    pulseLabel.append(this.pulseInput, el('span', '', 'Pulse outline'));
    this.navigation = el('div', 'scv-nav');
    this.previousButton = button('←', 'Previous change', () => this._step(-1));
    this.nextButton = button('→', 'Next change', () => this._step(1));
    this.position = el('span', 'scv-position', '–');
    this.position.setAttribute('aria-live', 'polite');
    this.navigation.append(this.previousButton, this.position, this.nextButton);
    this.toolbar.append(modes, groupLabel, pulseLabel, this.navigation);
    this.grid = el('div', 'scv-grid');
    this.footer = el('div', 'scv-footer');
    this.rangeLabel = el('span'); this.scopeLabel = el('span');
    this.footer.append(this.rangeLabel, this.scopeLabel);
    this.root.append(this.toolbar, this.grid, this.footer);
  }

  setContext({ sheet, change, variants, changes, names, notes, outcomes, motion = true }) {
    Object.assign(NAMES, names || {});
    this.notes = notes || {};
    this.outcomes = outcomes || {};
    if (this.destroyed) return;
    this.sheet = sheet;
    this.changes = Array.isArray(changes) ? changes : (sheet?.changes || []);
    this.change = change || this.changes[0];
    // Never silently introduce an unselected pipeline.
    this.variants = [...new Set((variants || ['original']).filter(id => NAMES[id]))];
    if (!this.variants.includes('original')) this.variants.unshift('original');

    this.setMotion(motion);
    if (!this.sheet || !this.change) {
      this.images.clear(); this.panes = []; this.grid.replaceChildren(el('div', 'scv-empty', 'No change selected.'));
      this.rangeLabel.textContent = ''; this.scopeLabel.textContent = '';
      this.previousButton.disabled = true; this.nextButton.disabled = true;
      return;
    }
    this.region = contextFor(sheet, this.change);
    this._prepareImages();
    this._buildPanes();
  }

  setMotion(value) {
    this.motion = Boolean(value);
    this._motionChanged();
  }

  _motionChanged() {
    const allowed = this.motion && !this.media?.matches;
    this.root.classList.toggle('scv-motion', allowed && this.pulse);
    if (this.pulseInput) {
      this.pulseInput.disabled = !allowed;
      this.pulseInput.checked = allowed && this.pulse;
    }
  }

  _prepareImages() {
    this.images.clear();
    const { w, h } = this.region;
    for (const variant of this.variants) {
      const canvas = nativeCanvas(w, h);
      const ctx = canvas.getContext('2d', { willReadFrequently: true });
      drawSheetRegion(ctx, this.sheet, this.region, { width: w, height: h, variant, quality: 'detail', highlight: false });
      this.images.set(variant, { variant, canvas, pixels: ctx.getImageData(0, 0, w, h).data });
    }
    const original = this.images.get('original');
    this.unionMask = new Uint8Array(w * h);
    this.pixelGroups = [];
    for (const item of this.images.values()) {
      item.mask = difference(original.pixels, item.pixels);
      for (let i = 0; i < item.mask.length; i++) this.unionMask[i] |= item.mask[i];
      const equal = this.pixelGroups.find(group => samePixels(group.image.pixels, item.pixels));
      if (equal) equal.variants.push(item.variant);
      else this.pixelGroups.push({ variants: [item.variant], image: item });
    }
    this.differentPixels = this.unionMask.reduce((sum, bit) => sum + bit, 0);
    for (const item of this.images.values()) {
      const mask = samePixels(item.pixels, original.pixels) ? this.unionMask : item.mask;
      item.displayMask = mask;
      item.diffCanvas = maskImage(item.pixels, mask, w, h);
      item.contour = contourImage(mask, w, h);
    }
    this.zoomRegion = this._zoomRegion();
  }

  _zoomRegion() {
    if (this.change.native && this.differentPixels) {
      return nativeZoomRegion(this.unionMask, this.region.w, this.region.h);
    }
    const clipped = intersect(this.change, this.region);
    const w = Math.max(1, Math.min(this.region.w, Math.min(360, clipped.w + 36)));
    const h = Math.max(1, Math.min(this.region.h, Math.min(160, clipped.h + 28)));
    let cx = clipped.x + clipped.w / 2, cy = clipped.y + clipped.h / 2;
    if (this.change.type === 'chart-color') cy = clipped.y + clipped.h * .59;
    return {
      x: Math.round(Math.max(this.region.x, Math.min(this.region.x + this.region.w - w, cx - w / 2))),
      y: Math.round(Math.max(this.region.y, Math.min(this.region.y + this.region.h - h, cy - h / 2))),
      w, h
    };
  }

  _buildPanes() {
    if (!this.sheet || !this.change || !this.images.size) return;
    this.grid.replaceChildren();
    this.panes = [];
    const groups = this.grouped ? this.pixelGroups : this.variants.map(variant => ({ variants: [variant], image: this.images.get(variant) }));
    this.grid.dataset.count = String(groups.length);
    for (const group of groups) {
      const pane = el('section', 'scv-pane');
      const header = el('div', 'scv-pane-head');
      const label = group.variants.map(id => NAMES[id]).join(' · ');
      const names = el('div', 'scv-names', label); names.title = label;
      const outcome = groupOutcome(group.variants, this.outcomes);
      header.dataset.outcome = outcome;
      header.title = { equal: 'No recorded difference', different: 'Recorded difference', unknown: 'Not assessed' }[outcome];
      names.prepend(el('span', 'scv-verdict', { equal: '✓ ', different: 'Δ ', unknown: '? ' }[outcome]));
      const isReference = group.variants.includes('original');
      let note = group.variants.length > 1 ? `${group.variants.length} variants · pixel-identical in this context` : isReference ? 'Reference · comparison locations highlighted' : 'Comparison with original';
      if (!this.differentPixels) note = 'Pixel-identical to original in this context';
      const evidenceNotes = group.variants.map(id => this.notes[id]).filter(Boolean);
      if (evidenceNotes.length) note += ' · ' + [...new Set(evidenceNotes)].join(' / ');
      header.append(names, el('div', 'scv-pane-note', note));
      const stage = el('div', 'scv-stage');
      const canvas = el('canvas', 'scv-image');
      const overlay = el('canvas', 'scv-emphasis');
      canvas.setAttribute('aria-label', `${label}: Context image and linked enlarged detail`);
      canvas.setAttribute('role', 'img');
      overlay.setAttribute('aria-hidden', 'true');
      stage.append(canvas, overlay); pane.append(header, stage); this.grid.append(pane);
      const entry = { group, pane, stage, canvas, overlay, layout: null };
      canvas.addEventListener('click', event => this._pick(event, entry));
      this.panes.push(entry);
    }
    const index = Math.max(0, this.changes.findIndex(item => item.id === this.change.id));
    this.position.textContent = `${index + 1} / ${Math.max(1, this.changes.length)}`;
    this.previousButton.disabled = this.changes.length < 2;
    this.nextButton.disabled = this.changes.length < 2;
    this.rangeLabel.textContent = `${formatRange(this.sheet, this.change)} · Context ${this.region.w} × ${this.region.h} original pixels`;
    this.scopeLabel.textContent = this.differentPixels ? `${groups.length} image${groups.length === 1 ? '' : 's'} from ${this.variants.length} variants · Equality applies to this context` : 'The available images are pixel-identical in this context.';
    this._schedule();
  }

  _setMask(value) {
    this.maskMode = Boolean(value);
    this.imagesButton.setAttribute('aria-pressed', String(!this.maskMode));
    this.maskButton.setAttribute('aria-pressed', String(this.maskMode));
    this._schedule();
  }

  _step(delta) {
    if (!this.changes?.length) return;
    const current = Math.max(0, this.changes.findIndex(change => change.id === this.change.id));
    this._select(this.changes[(current + delta + this.changes.length) % this.changes.length]);
  }

  _select(change) {
    if (!change || change.id === this.change?.id) return;
    this.setContext({ sheet: this.sheet, change, variants: this.variants, changes: this.changes, motion: this.motion });
    this.callbacks.onSelectChange?.(change);
  }

  _pick(event, entry) {
    if (!entry.layout) return;
    const bounds = entry.canvas.getBoundingClientRect();
    const px = event.clientX - bounds.left, py = event.clientY - bounds.top;
    const box = entry.layout.image;
    if (px < box.x || py < box.y || px > box.x + box.w || py > box.y + box.h) return;
    const x = this.region.x + (px - box.x) / box.scale;
    const y = this.region.y + (py - box.y) / box.scale;
    const match = this.changes.find(change => x >= change.x && x <= change.x + change.w && y >= change.y && y <= change.y + change.h);
    if (match) this._select(match);
  }

  _schedule() {
    if (this.destroyed) return;
    cancelAnimationFrame(this.frame);
    this.frame = requestAnimationFrame(() => { this.frame = 0; this._layout(); });
  }

  _layout() {
    if (this.destroyed || !this.panes.length) return;
    const rect = this.grid.getBoundingClientRect();
    if (rect.width < 5 || rect.height < 5) return;
    const count = this.panes.length;
    const portrait = rect.height > rect.width;
    let columns = count, rows = 1;
    if (count === 4) { columns = rect.width >= 410 ? 2 : 1; rows = Math.ceil(count / columns); }
    else if (portrait || (count > 1 && rect.width / count < 200)) { columns = 1; rows = count; }
    this.grid.style.gridTemplateColumns = `repeat(${columns}, minmax(0, 1fr))`;
    this.grid.style.gridTemplateRows = `repeat(${rows}, minmax(200px, 1fr))`;
    this.grid.dataset.orientation = portrait ? 'portrait' : 'landscape';
    const sizes = this.panes.map(entry => entry.stage.getBoundingClientRect());
    // One shared drawing size ensures identical image and magnifier scale.
    const sharedWidth = Math.min(...sizes.map(size => size.width));
    const sharedHeight = Math.min(...sizes.map(size => size.height));
    if (sharedWidth < 1 || sharedHeight < 1) return;
    const layout = this._imageLayout(sharedWidth, sharedHeight);
    for (const entry of this.panes) {
      const size = entry.stage.getBoundingClientRect();
      this._paint(entry, size.width, size.height, layout);
    }
  }

  _imageLayout(width, height) {
    const insetSide = width / Math.max(1, height) > 2;
    let image, zoom;
    if (insetSide) {
      // Short, wide panels get two reserved zones so neither image becomes
      // a tiny strip. Their source pixels still use the same uniform scale.
      const available = Math.max(40, width - 24);
      const mainWidth = available * .58;
      const gap = Math.min(20, available * .04);
      image = fitImage(this.region, { x: 12, y: 18, w: mainWidth, h: Math.max(35, height - 28) });
      zoom = fitImage(this.zoomRegion, {
        x: 12 + mainWidth + gap, y: 26,
        w: Math.max(20, available - mainWidth - gap), h: Math.max(35, height - 40)
      });
    } else {
      const insetHeight = Math.max(38, Math.min(130, height * .29));
      const mainHeight = Math.max(35, height - insetHeight - 48);
      image = fitImage(this.region, { x: 12, y: 18, w: Math.max(20, width - 24), h: mainHeight });
      zoom = fitImage(this.zoomRegion, { x: 12, y: height - insetHeight - 10, w: Math.max(20, width - 24), h: insetHeight });
    }
    // Anchor the inset to the right of its reserved zone.
    zoom.x = width - 12 - zoom.w;
    const focus = {
      x: image.x + (this.zoomRegion.x - this.region.x) * image.scale,
      y: image.y + (this.zoomRegion.y - this.region.y) * image.scale,
      w: this.zoomRegion.w * image.scale, h: this.zoomRegion.h * image.scale
    };
    return { width, height, image, zoom, focus, insetSide };
  }

  _paint(entry, width, height, sharedLayout) {
    const dpr = Math.min(2, window.devicePixelRatio || 1);
    const dx = (width - sharedLayout.width) / 2, dy = (height - sharedLayout.height) / 2;
    const layout = {
      ...sharedLayout,
      image: { ...sharedLayout.image, x: sharedLayout.image.x + dx, y: sharedLayout.image.y + dy },
      zoom: { ...sharedLayout.zoom, x: sharedLayout.zoom.x + dx, y: sharedLayout.zoom.y + dy },
      focus: { ...sharedLayout.focus, x: sharedLayout.focus.x + dx, y: sharedLayout.focus.y + dy }
    };
    entry.layout = layout;
    for (const canvas of [entry.canvas, entry.overlay]) {
      canvas.width = Math.max(1, Math.round(width * dpr));
      canvas.height = Math.max(1, Math.round(height * dpr));
    }
    const ctx = entry.canvas.getContext('2d');
    const overlay = entry.overlay.getContext('2d');
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    overlay.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.fillStyle = '#eef2ef'; ctx.fillRect(0, 0, width, height);
    const image = entry.group.image;
    const source = this.maskMode ? image.diffCanvas : image.canvas;
    const b = layout.image, z = layout.zoom, f = layout.focus;
    ctx.imageSmoothingEnabled = true;
    ctx.drawImage(source, b.x, b.y, b.w, b.h);
    ctx.strokeStyle = '#c6d2ca'; ctx.lineWidth = 1;
    ctx.strokeRect(b.x - .5, b.y - .5, b.w + 1, b.h + 1);

    // Two matching corners keep the magnifier spatially tied to its source.
    ctx.strokeStyle = this.differentPixels ? '#b56a60' : '#8aa297'; ctx.lineWidth = .9;
    ctx.beginPath();
    if (layout.insetSide) {
      ctx.moveTo(f.x + f.w, f.y); ctx.lineTo(z.x, z.y);
      ctx.moveTo(f.x + f.w, f.y + f.h); ctx.lineTo(z.x, z.y + z.h);
    } else {
      ctx.moveTo(f.x, f.y + f.h); ctx.lineTo(z.x, z.y);
      ctx.moveTo(f.x + f.w, f.y + f.h); ctx.lineTo(z.x + z.w, z.y);
    }
    ctx.stroke();
    ctx.fillStyle = '#fff'; ctx.fillRect(z.x - 2, z.y - 2, z.w + 4, z.h + 4);
    ctx.imageSmoothingEnabled = z.scale < 1;
    const zx = this.zoomRegion.x - this.region.x, zy = this.zoomRegion.y - this.region.y;
    ctx.drawImage(source, zx, zy, this.zoomRegion.w, this.zoomRegion.h, z.x, z.y, z.w, z.h);
    ctx.strokeStyle = this.differentPixels ? '#b8645a' : '#8aa297'; ctx.lineWidth = 1;
    ctx.strokeRect(z.x - 1, z.y - 1, z.w + 2, z.h + 2);

    ctx.font = '10px Inter, Segoe UI, Arial, sans-serif';
    ctx.textAlign = 'left'; ctx.textBaseline = 'middle';
    ctx.fillStyle = '#566c60';
    ctx.fillText(this.maskMode ? 'Context · changed pixels' : 'Context', 12 + dx, 9 + dy);
    const zoomLabel = `Detail · ${(z.scale / b.scale).toFixed(1).replace('.', ',')}×`;
    const labelWidth = ctx.measureText(zoomLabel).width + 8;
    ctx.fillStyle = '#eef2ef'; ctx.fillRect(z.x - 3, z.y - 17, labelWidth, 14);
    ctx.fillStyle = '#566c60'; ctx.fillText(zoomLabel, z.x, z.y - 9);

    if (this.differentPixels) {
      // Original images must retain the actual text and fill colours. Dense
      // per-pixel contours belong exclusively to the optional mask view.
      if (this.maskMode) {
        overlay.imageSmoothingEnabled = true;
        overlay.drawImage(image.contour, b.x, b.y, b.w, b.h);
        overlay.imageSmoothingEnabled = z.scale < 1;
        overlay.drawImage(image.contour, zx, zy, this.zoomRegion.w, this.zoomRegion.h, z.x, z.y, z.w, z.h);
      }
      overlay.strokeStyle = '#b9423c'; overlay.lineWidth = 1.2;
      overlay.strokeRect(f.x - 1.5, f.y - 1.5, f.w + 3, f.h + 3);
      const selected = intersect(this.change, this.region);
      if (selected.w > 0 && selected.h > 0) {
        overlay.setLineDash([3, 3]); overlay.strokeStyle = '#b9423c';
        overlay.strokeRect(b.x + (selected.x - this.region.x) * b.scale - 1.5, b.y + (selected.y - this.region.y) * b.scale - 1.5, selected.w * b.scale + 3, selected.h * b.scale + 3);
        overlay.setLineDash([]);
      }
    }
  }

  destroy() {
    if (this.destroyed) return;
    this.destroyed = true;
    cancelAnimationFrame(this.frame);
    this.observer.disconnect();
    this.media.removeEventListener?.('change', this.onMediaChange);
    this.images.clear();
    this.panes = [];
    this.root.remove();
    if (!this.hadHostClass) this.container.classList.remove('scv-host');
  }
}
