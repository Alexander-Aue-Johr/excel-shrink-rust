import { drawSheetRegion } from './sheet-renderer.js';

// Canvas dimensions are supplied by the caller. All internal geometry is in
// those canvas pixels; returned interaction rectangles are normalised to 0..1.
const C = {
  body: '#192a34', edge: '#40545e', text: '#d9e3e7', muted: '#93a8b4',
  tile: '#233741', tileEdge: '#405763', tileChanged: '#3a3039',
  tileChangedEdge: '#a4646d', red: '#df8f96', redFill: 'rgba(218,124,136,.15)',
  previewPadding: '#dfe7e5', paper: '#ffffff', neutralMap: '#293e48',
  neutralEdge: '#647c87', captureTint: 'rgba(104,195,167,.15)',
  green: '#8ad5ae', greenFill: 'rgba(115,210,166,.16)',
};
const FONT = '"Inter", "Segoe UI", system-ui, sans-serif';
const FORMAT = new Intl.NumberFormat('de-DE', { maximumFractionDigits: 2 });
const clamp = (n, lo, hi) => Math.max(lo, Math.min(hi, n));
const finite = (n, fallback = 0) => Number.isFinite(Number(n)) ? Number(n) : fallback;
const rect = (x, y, w, h) => ({ x, y, w, h });

function intersect(a, b) {
  const x = Math.max(a.x, b.x), y = Math.max(a.y, b.y);
  const right = Math.min(a.x + a.w, b.x + b.w), bottom = Math.min(a.y + a.h, b.y + b.h);
  return right > x && bottom > y ? rect(x, y, right - x, bottom - y) : null;
}

function normaliseSourceRect(input) {
  let x = finite(input?.x), y = finite(input?.y), w = finite(input?.w), h = finite(input?.h);
  if (w < 0) { x += w; w = -w; }
  if (h < 0) { y += h; h = -h; }
  return rect(x, y, w, h);
}

function inset(r, amount) {
  return rect(r.x + amount, r.y + amount, Math.max(.01, r.w - amount * 2), Math.max(.01, r.h - amount * 2));
}

function normalised(r, width, height) {
  const x = clamp(r.x / width, 0, 1), y = clamp(r.y / height, 0, 1);
  return { x, y, w: Math.max(0, clamp((r.x + r.w) / width, 0, 1) - x), h: Math.max(0, clamp((r.y + r.h) / height, 0, 1) - y) };
}

function expandedTarget(r, minimum, width, height) {
  const w = Math.min(width, Math.max(minimum, r.w));
  const h = Math.min(height, Math.max(minimum, r.h));
  return rect(clamp(r.x + r.w / 2 - w / 2, 0, width - w), clamp(r.y + r.h / 2 - h / 2, 0, height - h), w, h);
}

function project(part, source) {
  const overlap = intersect(source, part.sourceRect);
  if (!overlap) return null;
  const a = part.sourceRect, b = part.displayRect;
  return {
    sourceRect: overlap,
    displayRect: rect(b.x + (overlap.x - a.x) / a.w * b.w, b.y + (overlap.y - a.y) / a.h * b.h, overlap.w / a.w * b.w, overlap.h / a.h * b.h),
  };
}

function sourcePartition(total, n, i) {
  return i === n ? total : total >= n ? Math.floor(total * i / n) : total * i / n;
}

function px(value) { return FORMAT.format(value).replaceAll('.', '\u202f'); }

function shortPx(value) {
  for (const [divisor, suffix] of [[1e9, 'G'], [1e6, 'M'], [1e3, 'k']]) {
    if (value >= divisor && Number.isInteger(value / divisor)) return `${value / divisor}${suffix}`;
  }
  return px(value);
}

function coordinateLines(value, availableWidth, fontSize) {
  const text = shortPx(value);
  const limit = Math.max(3, Math.floor(availableWidth / (fontSize * .6)));
  if (text.length <= limit) return [text];
  const lines = [];
  let line = '';
  for (const group of text.split('\u202f')) {
    const candidate = line ? `${line}\u202f${group}` : group;
    if (line && candidate.length > limit) { lines.push(line); line = group; }
    else line = candidate;
  }
  if (line) lines.push(line);
  return lines;
}

function sheetDimensions(sheet) {
  const width = Number(sheet?.width), height = Number(sheet?.height);
  if (!(width > 0) || !(height > 0) || !Number.isFinite(width) || !Number.isFinite(height)) {
    throw new RangeError('A sheet needs finite positive width and height in original pixels.');
  }
  return { width, height };
}

function pipelineIds(value) {
  return (Array.isArray(value) ? value : value ? [value] : []).map(p => typeof p === 'string' ? p : p?.id).filter(Boolean);
}

function filteredChanges(sheet, pipelines, supplied) {
  if (Array.isArray(sheet.selectedChanges)) return sheet.selectedChanges;
  if (Array.isArray(supplied)) return supplied;
  const selected = pipelineIds(pipelines);
  return (Array.isArray(sheet.changes) ? sheet.changes : []).filter(change => {
    const signatures = change.signatures;
    const candidates = selected.length ? selected : signatures ? Object.keys(signatures).filter(id => id !== 'original') : change.pipelines || [];
    if (!candidates.length) return !signatures && !Array.isArray(change.pipelines);
    return candidates.some(id => {
      if (signatures && signatures.original !== undefined && signatures[id] !== undefined) return signatures[id] !== signatures.original;
      return Array.isArray(change.pipelines) ? change.pipelines.includes(id) : true;
    });
  });
}

function contain(source, target) {
  const scale = Math.min(target.w / source.w, target.h / source.h);
  return rect(target.x + (target.w - source.w * scale) / 2, target.y + (target.h - source.h * scale) / 2, source.w * scale, source.h * scale);
}

function shapeKind(sheet) {
  if (sheet.kind === 'wide' || sheet.kind === 'tall') return sheet.kind;
  return sheet.kind === 'square' || Math.max(sheet.width, sheet.height) > 5000 && Math.max(sheet.width, sheet.height) / Math.min(sheet.width, sheet.height) < 4 ? 'square' : 'normal';
}

function getMapParts(sheet, area, mode, unit) {
  if (!area) return [];
  const header = 12 * unit;
  const inside = rect(area.x + 4 * unit, area.y + header, area.w - 8 * unit, area.h - header - 4 * unit);
  const gap = 3 * unit, font = 8.5 * unit, cut = 2.5 * unit;
  const parts = [];
  if (mode === 'rows') {
    const label = 10 * unit;
    const maximum = clamp(Math.floor((inside.h + gap) / (label + 6 * unit + gap)), 2, 12);
    const ideal = Math.round(Math.sqrt(sheet.width / sheet.height * inside.h / Math.max(1, inside.w)));
    const count = clamp(ideal, Math.min(4, maximum), maximum);
    const pitch = (inside.h + gap) / count;
    for (let i = 0; i < count; i++) {
      const start = sourcePartition(sheet.width, count, i), end = sourcePartition(sheet.width, count, i + 1);
      parts.push({
        index: i, mode, cutBefore: i > 0, cutAfter: i < count - 1,
        sourceRect: rect(start, 0, end - start, sheet.height),
        displayRect: rect(inside.x + cut, inside.y + pitch * i + label, inside.w - cut * 2, Math.max(unit, pitch - label - gap)),
        startLabel: shortPx(start), endLabel: shortPx(end), fontSize: font, cut,
      });
    }
  } else if (mode === 'columns') {
    const maximum = clamp(Math.floor((inside.w + gap) / (18 * unit + gap)), 2, 12);
    const ideal = Math.round(Math.sqrt(sheet.height / sheet.width * inside.w / Math.max(1, inside.h)));
    const count = clamp(ideal, Math.min(4, maximum), maximum);
    const pitch = (inside.w + gap) / count, columnWidth = pitch - gap;
    const boundaries = Array.from({ length: count + 1 }, (_, i) => sourcePartition(sheet.height, count, i));
    const labels = boundaries.map(value => coordinateLines(value, columnWidth, font));
    const rows = Math.max(...labels.map(lines => lines.length));
    const labelHeight = rows * 9.5 * unit + 3 * unit;
    const mapY = inside.y + labelHeight + cut;
    const mapHeight = Math.max(unit, inside.h - labelHeight * 2 - cut * 2);
    for (let i = 0; i < count; i++) {
      parts.push({
        index: i, mode, cutBefore: i > 0, cutAfter: i < count - 1,
        sourceRect: rect(0, boundaries[i], sheet.width, boundaries[i + 1] - boundaries[i]),
        displayRect: rect(inside.x + pitch * i, mapY, columnWidth, mapHeight),
        startLines: labels[i], endLines: labels[i + 1], fontSize: font, cut,
      });
    }
  } else {
    const source = rect(0, 0, sheet.width, sheet.height);
    const target = inset(inside, 4 * unit);
    const display = contain(source, target);
    parts.push({ index: 0, mode: '2d', sourceRect: source, displayRect: display, cutBefore: false, cutAfter: false, fontSize: font, cut: 0 });
  }
  return parts;
}

/** Primary tile keeps W/H exactly. Its minimap is a flush, separate attachment. */
export function getSheetLayout(sheet, width, height, { previewRegion } = {}) {
  sheetDimensions(sheet);
  width = Math.max(1, finite(width, 1)); height = Math.max(1, finite(height, 1));
  const unit = Math.min(width, height) / 300;
  const kind = shapeKind(sheet);
  const thumbnailRect = rect(0, 0, width, height);
  let attachment = null, mapRect = null, mapMode = null;
  if (kind === 'wide') {
    mapMode = 'rows';
    attachment = { placement: 'bottom', x: 0, y: height, width, height: Math.max(1, Math.floor(height * .32)) };
  } else if (kind === 'tall' || kind === 'square') {
    mapMode = kind === 'tall' ? 'columns' : '2d';
    attachment = { placement: 'right', x: width, y: 0, width: Math.max(1, Math.floor(width * .32)), height };
  }

  const whole = rect(0, 0, sheet.width, sheet.height);
  let region = previewRegion ? intersect(normaliseSourceRect(previewRegion), whole) : null;
  if (!region) {
    if (kind === 'wide') region = rect(0, 0, Math.min(sheet.width, sheet.height * thumbnailRect.w / thumbnailRect.h), sheet.height);
    else if (kind === 'tall') region = rect(0, 0, sheet.width, Math.min(sheet.height, sheet.width * thumbnailRect.h / thumbnailRect.w));
    else if (kind === 'square') {
      const aspect = thumbnailRect.w / thumbnailRect.h;
      const cropHeight = Math.min(900, 1600 / aspect, sheet.height, sheet.width / aspect);
      region = rect(0, 0, cropHeight * aspect, cropHeight);
    } else region = whole;
  }
  const previewRect = contain(region, thumbnailRect);
  let mapParts = [];
  if (attachment) {
    attachment.widthRatio = attachment.width / width;
    attachment.heightRatio = attachment.height / height;
    attachment.mapRect = rect(0, 0, attachment.width, attachment.height);
    attachment.mapMode = mapMode;
    attachment.unit = unit;
    attachment.mapParts = getMapParts(sheet, attachment.mapRect, mapMode, unit);
    mapRect = rect(attachment.x, attachment.y, attachment.width, attachment.height);
    mapParts = attachment.mapParts.map(part => ({ ...part, displayRect: rect(part.displayRect.x + attachment.x, part.displayRect.y + attachment.y, part.displayRect.w, part.displayRect.h) }));
  }
  return {
    kind: 'sheet', shape: kind, width, height, unit, aspect: width / height,
    thumbnailRect, previewRect, previewRegion: region, mapRect, mapMode, mapParts, attachment,
    sourceScale: previewRect.w / region.w,
    completeSourceCoverage: mapParts.length ? mapParts.reduce((sum, p) => sum + p.sourceRect.w * p.sourceRect.h, 0) : sheet.width * sheet.height,
  };
}

/** Outer footprint ratios, before texture creation; attachments have no gap. */
export function getCardExtents(item, { mode = 'workbooks', aspect } = {}) {
  const isSheet = item.kind === 'sheet' || Boolean(item.sheet) || mode === 'sheets' || mode === 'sheet';
  const shape = isSheet ? shapeKind(item.sheet || item) : 'normal';
  const right = shape === 'tall' || shape === 'square' ? .32 : 0;
  const bottom = shape === 'wide' ? .32 : 0;
  return { right, bottom, widthRatio: 1 + right, heightRatio: 1 + bottom, primaryAspect: aspect };
}

function newCanvas(reference, width, height, factory) {
  let result;
  if (typeof factory === 'function') result = factory(width, height);
  else if (reference.ownerDocument?.createElement) result = reference.ownerDocument.createElement('canvas');
  else if (typeof OffscreenCanvas !== 'undefined') result = new OffscreenCanvas(width, height);
  else result = new reference.constructor(width, height);
  result.width = width; result.height = height;
  return result;
}

function roundedPath(ctx, r, radius) {
  const q = Math.max(0, Math.min(radius, r.w / 2, r.h / 2));
  ctx.beginPath(); ctx.moveTo(r.x + q, r.y); ctx.lineTo(r.x + r.w - q, r.y);
  ctx.quadraticCurveTo(r.x + r.w, r.y, r.x + r.w, r.y + q); ctx.lineTo(r.x + r.w, r.y + r.h - q);
  ctx.quadraticCurveTo(r.x + r.w, r.y + r.h, r.x + r.w - q, r.y + r.h); ctx.lineTo(r.x + q, r.y + r.h);
  ctx.quadraticCurveTo(r.x, r.y + r.h, r.x, r.y + r.h - q); ctx.lineTo(r.x, r.y + q);
  ctx.quadraticCurveTo(r.x, r.y, r.x + q, r.y); ctx.closePath();
}

function paperPath(ctx, part) {
  const { x, y, w, h } = part.displayRect, cut = part.cut;
  ctx.beginPath(); ctx.moveTo(x, y);
  if (part.mode === 'rows') {
    ctx.lineTo(x + w, y);
    if (part.cutAfter) {
      const steps = Math.max(2, Math.ceil(h / Math.max(1, cut * 1.5)));
      for (let i = 1; i <= steps; i++) ctx.lineTo(x + w + (i < steps && i % 2 ? cut : 0), y + h * i / steps);
    } else ctx.lineTo(x + w, y + h);
    ctx.lineTo(x, y + h);
    if (part.cutBefore) {
      const steps = Math.max(2, Math.ceil(h / Math.max(1, cut * 1.5)));
      for (let i = 1; i <= steps; i++) ctx.lineTo(x - (i < steps && i % 2 ? cut : 0), y + h - h * i / steps);
    } else ctx.lineTo(x, y);
  } else if (part.mode === 'columns') {
    if (part.cutBefore) {
      const steps = Math.max(2, Math.ceil(w / Math.max(1, cut * 2)));
      for (let i = 1; i <= steps; i++) ctx.lineTo(x + w * i / steps, y - (i < steps && i % 2 ? cut : 0));
    } else ctx.lineTo(x + w, y);
    ctx.lineTo(x + w, y + h);
    if (part.cutAfter) {
      const steps = Math.max(2, Math.ceil(w / Math.max(1, cut * 2)));
      for (let i = 1; i <= steps; i++) ctx.lineTo(x + w - w * i / steps, y + h + (i < steps && i % 2 ? cut : 0));
    } else ctx.lineTo(x, y + h);
    ctx.lineTo(x, y);
  } else { ctx.lineTo(x + w, y); ctx.lineTo(x + w, y + h); ctx.lineTo(x, y + h); }
  ctx.closePath();
}

function textWidth(ctx, text) { return ctx.measureText(text).width; }

function ellipsis(ctx, text, width) {
  if (textWidth(ctx, text) <= width) return text;
  let lo = 0, hi = text.length;
  while (lo < hi) {
    const middle = Math.ceil((lo + hi) / 2);
    if (textWidth(ctx, `${text.slice(0, middle)}…`) <= width) lo = middle;
    else hi = middle - 1;
  }
  return `${text.slice(0, lo)}…`;
}

function wrappedName(ctx, text, width, maxLines = 2) {
  const words = String(text).split(/\s+/), lines = [];
  let line = '';
  for (let i = 0; i < words.length; i++) {
    const candidate = line ? `${line} ${words[i]}` : words[i];
    if (line && textWidth(ctx, candidate) > width) {
      lines.push(ellipsis(ctx, line, width)); line = words[i];
      if (lines.length === maxLines - 1) { line += words.length > i + 1 ? ` ${words.slice(i + 1).join(' ')}` : ''; break; }
    } else line = candidate;
  }
  if (line && lines.length < maxLines) lines.push(ellipsis(ctx, line, width));
  return lines;
}

function beginBody(canvas) {
  const width = Math.max(1, canvas.width), height = Math.max(1, canvas.height);
  const ctx = canvas.getContext('2d');
  if (!ctx) throw new Error('A 2D canvas context is required.');
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.clearRect(0, 0, width, height);
  ctx.fillStyle = C.body;
  ctx.fillRect(0, 0, width, height);
  return { ctx, width, height, unit: Math.min(width, height) / 300 };
}

function gridLayout(count, width, height, aspect, unit) {
  const padding = 9 * unit, gap = 6 * unit;
  const availableWidth = width - padding * 2, availableHeight = height - padding * 2;
  let best = null;
  for (let columns = 1; columns <= count; columns++) {
    const rows = Math.ceil(count / columns);
    const tileWidth = Math.min((availableWidth - (columns - 1) * gap) / columns, (availableHeight - (rows - 1) * gap) / rows * aspect);
    if (tileWidth <= 0) continue;
    const tileHeight = tileWidth / aspect;
    const score = tileWidth * tileHeight * count * (1 - .01 * (columns * rows - count) / count);
    if (!best || score > best.score) best = { columns, rows, tileWidth, tileHeight, score };
  }
  if (!best) {
    // Very dense input remains complete. Gutters collapse before any tile is omitted.
    const columns = Math.ceil(Math.sqrt(count)), rows = Math.ceil(count / columns);
    const tileWidth = Math.min(width / columns, height / rows * aspect);
    best = { columns, rows, tileWidth, tileHeight: tileWidth / aspect, score: 0 };
    best.gap = 0;
  }
  const actualGap = best.gap ?? gap;
  const usedWidth = best.columns * best.tileWidth + (best.columns - 1) * actualGap;
  const usedHeight = best.rows * best.tileHeight + (best.rows - 1) * actualGap;
  return { ...best, gap: actualGap, x: (width - usedWidth) / 2, y: (height - usedHeight) / 2 };
}

function paintWorkbookCard(canvas, item, options) {
  const { ctx, width, height, unit } = beginBody(canvas);
  const sheets = item.sheets ?? item.workbook?.allSheets ?? item.workbook?.sheets ?? [];
  const pipelines = pipelineIds(options.pipelines?.length ? options.pipelines : item.pipelines);
  const aspect = finite(options.aspect, width / height) > 0 ? finite(options.aspect, width / height) : width / height;
  if (!sheets.length) {
    ctx.fillStyle = C.muted; ctx.font = `${12 * unit}px ${FONT}`; ctx.textAlign = 'center';
    ctx.fillText('Keine Arbeitsblätter', width / 2, height / 2);
    return { hits: [], attachments: [], layoutInfo: { kind: 'workbook', width, height, tileAspect: aspect, sheetCount: 0, tiles: [] } };
  }
  const grid = gridLayout(sheets.length, width, height, aspect, unit);
  const hits = [], tiles = [];
  sheets.forEach((sheet, index) => {
    const tile = rect(grid.x + index % grid.columns * (grid.tileWidth + grid.gap), grid.y + Math.floor(index / grid.columns) * (grid.tileHeight + grid.gap), grid.tileWidth, grid.tileHeight);
    const count = filteredChanges(sheet, pipelines).length;
    roundedPath(ctx, tile, Math.min(5 * unit, tile.w * .06));
    ctx.fillStyle = count ? C.tileChanged : C.tile; ctx.fill();
    ctx.strokeStyle = count ? C.tileChangedEdge : C.tileEdge; ctx.lineWidth = Math.max(.5, .8 * unit); ctx.stroke();
    const tilePad = Math.min(7 * unit, tile.w * .09, tile.h * .13);
    const indexSize = Math.min(12 * unit, tile.h * .22, tile.w * .23);
    const nameSize = Math.min(10.5 * unit, tile.h * .18, tile.w * .16);
    const label = sheet.name || sheet.title || `Blatt ${index + 1}`;
    ctx.font = `500 ${indexSize}px ${FONT}`; ctx.textBaseline = 'top'; ctx.textAlign = 'left';
    ctx.fillStyle = count ? C.red : C.muted;
    ctx.fillText(String(index + 1).padStart(2, '0'), tile.x + tilePad, tile.y + tilePad);
    if (count && tile.w > 38 * unit && tile.h > 25 * unit) {
      ctx.textAlign = 'right'; ctx.fillText(String(count), tile.x + tile.w - tilePad, tile.y + tilePad);
    }
    if (tile.h > 30 * unit && tile.w > 32 * unit) {
      ctx.font = `${nameSize}px ${FONT}`; ctx.fillStyle = count ? C.text : C.muted; ctx.textAlign = 'left';
      const lines = wrappedName(ctx, label, tile.w - tilePad * 2, 2);
      const lineHeight = nameSize * 1.2;
      const y = Math.max(tile.y + tilePad + indexSize + 4 * unit, tile.y + (tile.h - lines.length * lineHeight) / 2 + 5 * unit);
      lines.forEach((line, i) => ctx.fillText(line, tile.x + tilePad, y + i * lineHeight));
    }
    const hit = { ...normalised(tile, width, height), kind: 'sheet', sheetId: sheet.id, label, changedCount: count, priority: 10 };
    hits.push(hit); tiles.push({ ...tile, sheetId: sheet.id, changedCount: count });
  });
  return { hits, attachments: [], layoutInfo: { kind: 'workbook', width, height, tileAspect: aspect, sheetCount: sheets.length, columns: grid.columns, rows: grid.rows, tiles } };
}

function drawMapLabels(ctx, layout) {
  const { mapRect: area, mapMode: mode, mapParts: parts, unit } = layout;
  if (!area) return;
  ctx.fillStyle = C.muted; ctx.font = `${9 * unit}px ${FONT}`; ctx.textAlign = 'left'; ctx.textBaseline = 'top';
  ctx.fillText(mode === 'rows' ? 'Gesamtblatt · x → · schematisch' : mode === 'columns' ? 'Gesamtblatt · y ↓' : 'Gesamtblatt', area.x + 4 * unit, area.y + unit);
  for (const part of parts) {
    const r = part.displayRect;
    ctx.font = `${part.fontSize}px ${FONT}`;
    ctx.fillStyle = C.muted;
    if (mode === 'rows') {
      ctx.textAlign = 'left'; ctx.textBaseline = 'alphabetic';
      ctx.fillText(`${part.index + 1} → ${part.startLabel}–${part.endLabel}`, r.x, r.y - 2 * unit);
    } else if (mode === 'columns') {
      ctx.textAlign = 'center'; ctx.textBaseline = 'alphabetic';
      part.startLines.forEach((line, i) => ctx.fillText(line, r.x + r.w / 2, r.y - part.cut - 3 * unit - (part.startLines.length - i - 1) * 9.5 * unit));
      part.endLines.forEach((line, i) => ctx.fillText(line, r.x + r.w / 2, r.y + r.h + part.cut + 8.5 * unit + i * 9.5 * unit));
    }
  }
}

function drawMappedRegion(ctx, part, region, color, fill, unit, ringWhenTiny = false) {
  const projected = project(part, region);
  if (!projected) return null;
  const r = projected.displayRect;
  ctx.save(); ctx.beginPath(); ctx.rect(part.displayRect.x, part.displayRect.y, part.displayRect.w, part.displayRect.h); ctx.clip();
  if (fill) { ctx.fillStyle = fill; ctx.fillRect(r.x, r.y, r.w, r.h); }
  ctx.strokeStyle = color; ctx.lineWidth = Math.max(.5, unit);
  ctx.strokeRect(r.x, r.y, r.w, r.h);
  ctx.restore();
  if (ringWhenTiny && (r.w < 4 * unit || r.h < 4 * unit)) {
    // A separate circular locator; the true mapped rectangle above stays unchanged.
    ctx.beginPath(); ctx.ellipse(r.x + r.w / 2, r.y + r.h / 2, 3.8 * unit, 3.8 * unit, 0, 0, Math.PI * 2);
    ctx.strokeStyle = C.body; ctx.lineWidth = 2.8 * unit; ctx.stroke();
    ctx.strokeStyle = color; ctx.lineWidth = 1.1 * unit; ctx.stroke();
  }
  return projected;
}

function changeHit(change, displayed, layout, sheetId) {
  const hitRect = expandedTarget(displayed, 17 * layout.unit, layout.width, layout.height);
  return {
    ...normalised(hitRect, layout.width, layout.height), kind: 'change', sheetId, changeId: change.id,
    sourcePoint: { x: change.x + change.w / 2, y: change.y + change.h / 2 },
    sourceRect: normaliseSourceRect(change), displayRect: normalised(displayed, layout.width, layout.height),
    priority: 100, label: change.label || 'Abweichung',
  };
}

function paintMapAttachment(canvas, sheet, sheetLayout, changes, options) {
  const { ctx, width, height } = beginBody(canvas);
  const layout = { ...sheetLayout.attachment, kind: 'map', width, height, previewRegion: sheetLayout.previewRegion };
  const { unit, mapParts, previewRegion } = layout;
  const hits = [];
  const capturedRegions = [previewRegion, ...(Array.isArray(options.capturedRegions) ? options.capturedRegions.map(normaliseSourceRect) : [])];
  for (const part of mapParts) {
    // Neutral geometry denotes unknown imagery. Jagged caps are outside its
    // exact affine rectangle, so no original source interval is omitted.
    paperPath(ctx, part); ctx.fillStyle = C.neutralMap; ctx.fill();
    ctx.strokeStyle = C.neutralEdge; ctx.lineWidth = Math.max(.5, .75 * unit); ctx.stroke();
    for (const capture of capturedRegions) {
      const r = project(part, capture);
      if (r) { ctx.fillStyle = C.captureTint; ctx.fillRect(r.displayRect.x, r.displayRect.y, r.displayRect.w, r.displayRect.h); }
    }
    drawMappedRegion(ctx, part, previewRegion, C.green, C.greenFill, unit, true);
    hits.push({
      ...normalised(part.displayRect, width, height), kind: 'map', sheetId: sheet.id,
      sourceRect: { ...part.sourceRect }, displayRect: normalised(part.displayRect, width, height),
      sourcePoint: { x: part.sourceRect.x + part.sourceRect.w / 2, y: part.sourceRect.y + part.sourceRect.h / 2 }, priority: 0,
    });
    for (const change of changes) {
      const r = drawMappedRegion(ctx, part, normaliseSourceRect(change), C.red, C.redFill, unit, false);
      if (!r) continue;
      const displayed = r.displayRect;
      if (displayed.w < 4 * unit || displayed.h < 4 * unit) {
        const cx = displayed.x + displayed.w / 2, cy = displayed.y + displayed.h / 2;
        ctx.beginPath(); ctx.moveTo(cx, cy - 2.5 * unit); ctx.lineTo(cx + 2.5 * unit, cy);
        ctx.lineTo(cx, cy + 2.5 * unit); ctx.lineTo(cx - 2.5 * unit, cy); ctx.closePath();
        ctx.fillStyle = C.red; ctx.fill();
      }
      hits.push(changeHit(change, displayed, layout, sheet.id));
    }
  }
  drawMapLabels(ctx, layout);
  hits.sort((a, b) => b.priority - a.priority || a.w * a.h - b.w * b.h);
  return {
    canvas, placement: layout.placement, widthRatio: layout.widthRatio, heightRatio: layout.heightRatio,
    hits, layoutInfo: { ...layout, selectedChangeCount: changes.length, unknownMiniatureImagery: true, thumbnailRenderCount: 0 },
  };
}

/** Draws only the available thumbnail image. The minimap NEVER calls the renderer. */
export function paintSheetCard(canvas, sheet, options = {}) {
  const { ctx, width, height } = beginBody(canvas);
  const layout = getSheetLayout(sheet, width, height, { previewRegion: options.previewRegion });
  const { unit, previewRect: imageRect, thumbnailRect, previewRegion } = layout;
  const pipelines = pipelineIds(options.pipelines);
  const changes = filteredChanges(sheet, pipelines, options.changes);
  const hits = [];
  ctx.fillStyle = sheet.backgroundColor || C.paper;
  ctx.fillRect(thumbnailRect.x, thumbnailRect.y, thumbnailRect.w, thumbnailRect.h);
  ctx.save();
  ctx.beginPath(); ctx.rect(imageRect.x, imageRect.y, imageRect.w, imageRect.h); ctx.clip();
  ctx.translate(imageRect.x, imageRect.y);
  drawSheetRegion(ctx, sheet, previewRegion, { width: imageRect.w, height: imageRect.h, quality: 'thumbnail', variant: options.variant || 'original' });
  ctx.restore();
  ctx.strokeStyle = '#a9bdbd'; ctx.lineWidth = Math.max(.5, .8 * unit);
  ctx.strokeRect(imageRect.x, imageRect.y, imageRect.w, imageRect.h);

  const imagePart = { sourceRect: previewRegion, displayRect: imageRect };
  for (const change of changes) {
    const r = drawMappedRegion(ctx, imagePart, normaliseSourceRect(change), C.red, null, unit, false);
    if (r) hits.push(changeHit(change, r.displayRect, layout, sheet.id));
  }
  hits.push({
    ...normalised(imageRect, width, height), kind: 'preview', sheetId: sheet.id,
    sourceRect: { ...previewRegion }, displayRect: normalised(imageRect, width, height),
    sourcePoint: { x: previewRegion.x + previewRegion.w / 2, y: previewRegion.y + previewRegion.h / 2 }, priority: 20,
  });

  const attachments = [];
  if (layout.attachment) {
    const a = layout.attachment;
    const mapCanvas = newCanvas(canvas, a.width, a.height, options.createCanvas);
    attachments.push(paintMapAttachment(mapCanvas, sheet, layout, changes, options));
  }
  hits.sort((a, b) => b.priority - a.priority || a.w * a.h - b.w * b.h);
  return { hits, attachments, previewRegion: { ...previewRegion }, layoutInfo: { ...layout, selectedChangeCount: changes.length, unknownMiniatureImagery: Boolean(layout.attachment), thumbnailRenderCount: 1 } };
}

/** Shared gallery/focus renderer. Titles and workbook names belong above the body. */
export function paintCard(canvas, item, { mode = 'workbooks', aspect, pipelines = [], ...options } = {}) {
  const isSheet = item.kind === 'sheet' || Boolean(item.sheet) || mode === 'sheets' || mode === 'sheet';
  const selectedPipelines = pipelines.length ? pipelines : item.pipelines || [];
  if (!isSheet) return paintWorkbookCard(canvas, item, { ...options, aspect, pipelines: selectedPipelines });
  const sheet = item.sheet || item;
  return paintSheetCard(canvas, sheet, {
    ...options,
    pipelines: selectedPipelines,
    changes: item.changes ?? options.changes,
    previewRegion: item.previewRegion ?? options.previewRegion,
    capturedRegions: item.capturedRegions ?? options.capturedRegions,
  });
}

/** Free navigation uses the exact display rectangle, never an enlarged target. */
export function sourcePointForHit(hit, bodyX, bodyY) {
  if (hit.kind === 'change') return hit.sourcePoint ? { ...hit.sourcePoint } : null;
  if (!hit.sourceRect || !hit.displayRect) return hit.sourcePoint ? { ...hit.sourcePoint } : null;
  const a = hit.displayRect, b = hit.sourceRect;
  const u = a.w ? clamp((bodyX - a.x) / a.w, 0, 1) : .5;
  const v = a.h ? clamp((bodyY - a.y) / a.h, 0, 1) : .5;
  return { x: b.x + u * b.w, y: b.y + v * b.h };
}
