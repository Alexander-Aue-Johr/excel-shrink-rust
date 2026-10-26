// Geometry-only capture glyphs. No synthetic tiles, timestamps, or image decoding.
// The lane's x axis always represents wall-clock time; dense events share bins.
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

export function capturePackages(events, now, windowMs, width, compact = false) {
  if (!Number.isFinite(now) || windowMs <= 0 || width <= 0) return [];
  const cellWidth = compact ? 24 : 34;
  const columns = Math.max(1, Math.floor(width / cellWidth));
  const buckets = new Map();
  for (const event of events || []) {
    if (!Number.isFinite(event.at) || event.at < now - windowMs || event.at > now) continue;
    const position = Math.min(columns - 1, Math.max(0, Math.floor((event.at - (now - windowMs)) / windowMs * columns)));
    const bucket = buckets.get(position) || { position, events: [], earliest: event.at, latest: event.at };
    bucket.events.push(event);
    bucket.earliest = Math.min(bucket.earliest, event.at);
    bucket.latest = Math.max(bucket.latest, event.at);
    buckets.set(position, bucket);
  }
  const ordered = [...buckets.values()].sort((a, b) => a.position - b.position);
  return ordered.map((bucket, index) => ({ ...bucket, x: (bucket.position + .5) * width / columns,
    generation: ordered.length - 1 - index, count: bucket.events.length }));
}

// Draw only source-backed positions. A missing/ambiguous location is represented
// by a count, never by an invented worksheet arrangement.
export function captureGeometry(events) {
  const candidates = (events || []).map(event => ({
    sheet: event.details?.sheet_index ?? event.details?.sheet_name,
    rect: parseA1Range(event.details?.range),
  }));
  if (!candidates.length || candidates.some(candidate => !candidate.rect || candidate.sheet == null)) return null;
  if (new Set(candidates.map(candidate => String(candidate.sheet))).size !== 1) return null;
  const left = Math.min(...candidates.map(item => item.rect.x));
  const top = Math.min(...candidates.map(item => item.rect.y));
  const right = Math.max(...candidates.map(item => item.rect.x + item.rect.width));
  const bottom = Math.max(...candidates.map(item => item.rect.y + item.rect.height));
  return candidates.map(item => ({
    x: (item.rect.x - left) / (right - left), y: (item.rect.y - top) / (bottom - top),
    width: item.rect.width / (right - left), height: item.rect.height / (bottom - top),
  }));
}

export function captureDescription(pack) {
  const locations = [...new Set(pack.events.map(event => [event.details?.sheet_name,
    event.details?.range].filter(Boolean).join('!')).filter(Boolean))];
  const when = new Date(pack.latest).toLocaleTimeString('en-US');
  return `${pack.count} Capture${pack.count === 1 ? '' : 's'} · ${when}${locations.length ? ' · ' + locations.slice(0, 3).join(', ') : ''}${locations.length > 3 ? ' …' : ''}`;
}
