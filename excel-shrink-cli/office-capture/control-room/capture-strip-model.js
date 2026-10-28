// Geometry-only capture summaries. `at` is the host's image receipt timestamp,
// not an inferred time at which Excel captured the image.
export function parseA1Range(value) {
  if (typeof value !== 'string') return null;
  const range = value.trim().split('!').at(-1);
  const match = /^\$?([A-Z]{1,3})\$?(\d+)(?::\$?([A-Z]{1,3})\$?(\d+))?$/i.exec(range);
  if (!match) return null;
  const column = text => [...text.toUpperCase()].reduce((number, letter) => number * 26 + letter.charCodeAt(0) - 64, 0) - 1;
  const left = column(match[1]), right = column(match[3] || match[1]);
  const top = Number(match[2]) - 1, bottom = Number(match[4] || match[2]) - 1;
  if (top < 0 || bottom < top || left < 0 || right < left || right > 16383 || bottom > 1048575) return null;
  return { x: left, y: top, width: right - left + 1, height: bottom - top + 1 };
}

function sheetIdentity(event) {
  const details = event?.details || {};
  const index = Number.isInteger(details.sheet_index) && details.sheet_index >= 0 ? details.sheet_index : null;
  const name = typeof details.sheet_name === 'string' && details.sheet_name ? details.sheet_name : null;
  return index == null && name == null ? null : JSON.stringify([index, name]);
}
function recentEvents(events, now, windowMs) {
  if (!Number.isFinite(now) || !Number.isFinite(windowMs) || windowMs <= 0) return [];
  return (events || []).filter(event => Number.isFinite(event?.at)
    && event.at >= now - windowMs && event.at <= now).sort((a, b) => a.at - b.at);
}
function boundsFor(events) {
  const rectangles = events.map(event => parseA1Range(event.details?.range)).filter(Boolean);
  if (!rectangles.length) return null;
  const x = Math.min(...rectangles.map(rect => rect.x)), y = Math.min(...rectangles.map(rect => rect.y));
  return { x, y, width: Math.max(...rectangles.map(rect => rect.x + rect.width)) - x,
    height: Math.max(...rectangles.map(rect => rect.y + rect.height)) - y };
}

// command_id identifies real capture jobs/batches. Without it, a fixed one-second
// receipt window is an explicitly visual group, never presented as a real batch.
export function captureGroups(events, now, windowMs = 60000) {
  const groups = new Map();
  for (const event of recentEvents(events, now, windowMs)) {
    const sheetKey = sheetIdentity(event);
    const command = event.details?.command_id;
    const commandId = typeof command === 'string' && command ? command
      : Number.isFinite(command) ? String(command) : null;
    const grouping = commandId == null ? 'time-window' : 'command';
    const id = JSON.stringify([grouping, commandId ?? Math.floor(event.at / 1000), sheetKey]);
    const group = groups.get(id) || { id, grouping, commandId, sheetKey,
      sheetName: event.details?.sheet_name || (event.details?.sheet_index != null ? `Sheet ${event.details.sheet_index}` : null),
      events: [], earliest: event.at, latest: event.at, count: 0 };
    group.events.push(event); group.count++;
    group.latest = Math.max(group.latest, event.at);
    groups.set(id, group);
  }
  return [...groups.values()].sort((a, b) => a.latest - b.latest || a.earliest - b.earliest);
}

// Use one coordinate basis for all layers and cards. A tile's worksheet position
// must not jump merely because a later packet contains a different set of ranges.
export function captureGeometry(events, bounds = null) {
  const candidates = (events || []).map(event => ({ event, sheet: sheetIdentity(event),
    rect: parseA1Range(event.details?.range) }));
  if (!candidates.length || candidates.some(item => !item.rect || item.sheet == null)) return null;
  if (new Set(candidates.map(item => item.sheet)).size !== 1) return null;
  bounds ||= boundsFor(events);
  if (!bounds || ![bounds.x, bounds.y, bounds.width, bounds.height].every(Number.isFinite)
    || bounds.width <= 0 || bounds.height <= 0) return null;
  return candidates.map(({ event, rect }) => ({ event,
    x: (rect.x - bounds.x) / bounds.width, y: (rect.y - bounds.y) / bounds.height,
    width: rect.width / bounds.width, height: rect.height / bounds.height }));
}
function frameFor(group, groups, bounds) {
  const sameSheet = group.sheetKey == null ? [group] : groups.filter(item => item.sheetKey === group.sheetKey);
  const position = sameSheet.findIndex(item => item.id === group.id);
  const prior = sameSheet.slice(Math.max(0, position - 2), position + 1).reverse();
  const layers = prior.map((item, generation) => {
    const located = item.events.filter(event => sheetIdentity(event) != null && parseA1Range(event.details?.range));
    const rects = captureGeometry(located, bounds);
    return { generation, id: item.id, events: item.events, rects,
      unlocatedCount: item.events.length - (rects?.length || 0) };
  });
  const unlocatedCount = layers.reduce((count, layer) => count + layer.unlocatedCount, 0);
  return { ...group, bounds, layers, unlocatedCount,
    geometryAvailable: layers.some(layer => layer.rects?.length), partialGeometry: unlocatedCount > 0 };
}

// The detail view follows the latest worksheet and retains the newest three
// logical capture groups. Each card shows its own new, previous and older tiles.
export function captureFrames(events, { now = Date.now(), windowMs = 60000, limit = 3 } = {}) {
  const groups = captureGroups(events, now, windowMs);
  const latest = groups.at(-1);
  if (!latest) return [];
  const sameSheet = latest.sheetKey == null ? [latest] : groups.filter(group => group.sheetKey === latest.sheetKey);
  const count = Number.isSafeInteger(limit) && limit > 0 ? Math.min(limit, 3) : 3;
  const shown = sameSheet.slice(-count);
  const context = sameSheet.slice(Math.max(0, sameSheet.length - shown.length - 2));
  const bounds = boundsFor(context.flatMap(group => group.events));
  return shown.map(group => frameFor(group, sameSheet, bounds));
}

const GLYPH_DIMENSIONS = {
  compact: {
    small: { width: 20, height: 17, spacing: 25 },
    medium: { width: 27, height: 23, spacing: 34 },
    large: { width: 34, height: 28, spacing: 42 },
  },
  regular: {
    small: { width: 24, height: 20, spacing: 30 },
    medium: { width: 34, height: 28, spacing: 42 },
    large: { width: 46, height: 38, spacing: 56 },
  },
};

export function captureGlyphDimensions(compact, size = 'large') {
  const dimensions = compact ? GLYPH_DIMENSIONS.compact : GLYPH_DIMENSIONS.regular;
  return dimensions[size] || dimensions.large;
}

// Screen-sized groups only avoid overlapping glyphs. Real groups and their
// receipt times survive aggregation; generations never depend on screen space.
export function capturePackages(events, now, windowMs, width, compact = false, glyphSize = 'large') {
  if (!Number.isFinite(width) || width <= 0) return [];
  const groups = captureGroups(events, now, windowMs);
  const dimensions = captureGlyphDimensions(compact, glyphSize);
  const glyphSpacing = dimensions.spacing;
  const glyphHalfWidth = dimensions.width / 2;
  const bounds = new Map();
  for (const group of groups) if (group.sheetKey != null && !bounds.has(group.sheetKey)) {
    bounds.set(group.sheetKey, boundsFor(groups.filter(item => item.sheetKey === group.sheetKey).flatMap(item => item.events)));
  }
  const buckets = [];
  for (const group of groups) {
    const x = (group.latest - (now - windowMs)) / windowMs * width;
    const displayX = width <= glyphHalfWidth * 2 ? width / 2 : Math.max(glyphHalfWidth, Math.min(width - glyphHalfWidth, x));
    let bucket = buckets.at(-1);
    if (!bucket || displayX - bucket.displayX >= glyphSpacing) {
      bucket = { position: buckets.length, x, displayX, groups: [], events: [] };
      buckets.push(bucket);
    }
    bucket.x = x; bucket.displayX = displayX;
    bucket.groups.push(group); bucket.events.push(...group.events);
  }
  const ordered = buckets;
  return ordered.map((bucket, index) => {
    const last = bucket.groups.at(-1);
    const sameSheet = last.sheetKey != null && bucket.groups.every(group => group.sheetKey === last.sheetKey);
    const earliest = Math.min(...bucket.events.map(event => event.at));
    const latest = Math.max(...bucket.events.map(event => event.at));
    return { ...bucket, earliest, latest, x: (latest - (now - windowMs)) / windowMs * width,
      generation: ordered.length - 1 - index, count: bucket.events.length, groupCount: bucket.groups.length,
      aggregation: bucket.groups.length > 1 ? 'display-bin' : last.grouping,
      frame: sameSheet ? frameFor(last, groups, bounds.get(last.sheetKey)) : null };
  });
}

export function captureDescription(pack) {
  const locations = [...new Set(pack.events.map(event => [event.details?.sheet_name,
    event.details?.range].filter(Boolean).join('!')).filter(Boolean))];
  const when = new Date(pack.latest).toLocaleTimeString('en-US');
  const grouping = pack.aggregation === 'display-bin' ? ` · ${pack.groupCount} groups combined for display`
    : (pack.grouping || pack.aggregation) === 'time-window' ? ' · visual 1 s receipt group' : ' · capture job';
  return `${pack.count} capture${pack.count === 1 ? '' : 's'} · received ${when}${grouping}${locations.length ? ' · ' + locations.slice(0, 3).join(', ') : ''}${locations.length > 3 ? ' …' : ''}`;
}
