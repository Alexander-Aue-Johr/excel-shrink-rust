// Pure view model: every rectangle and equality class comes from a real job.
export const PRIMARY_PROFILES = [
  { id: 'original', label: 'Original' },
  { id: 'default', label: 'Default' },
  { id: 'maximum-cleaning', label: 'Maximum cleaning' },
  { id: 'too-aggressive-cleaning', label: 'Too aggressive' },
];

function versioned(url, version) {
  if (!url || version == null) return url;
  return `${url}${url.includes('?') ? '&' : '?'}v=${encodeURIComponent(version)}`;
}

export function normalizeGroup(raw) {
  const profiles = (raw.profiles?.length ? raw.profiles : PRIMARY_PROFILES).map(profile =>
    typeof profile === 'string' ? { id: profile, label: PRIMARY_PROFILES.find(p => p.id === profile)?.label || profile } : profile);
  const sheets = (raw.sheets || []).map((sheet, index) => {
    const id = String(sheet.id ?? sheet.index ?? index);
    let nextY = 0;
    const tiles = (sheet.tiles || []).map((tile, tileIndex) => {
      const images = Object.fromEntries(Object.entries(tile.images || {}).map(([profile, image]) => [profile, {
        ...image,
        url: versioned(image.url || image.preview_url || image.previewUrl, image.updated_at ?? tile.updated_at),
        previewUrl: versioned(image.preview_url || image.previewUrl || image.url, image.updated_at ?? tile.updated_at),
        fullUrl: versioned(image.full_url || image.fullUrl || (image.retainedFullResolution ? image.url : null), image.updated_at ?? tile.updated_at),
        pixelClass: image.pixel_class ?? image.pixelClass ?? null,
        maskUrl: versioned(image.mask_url || image.maskUrl, image.updated_at ?? tile.updated_at),
        changedPixels: image.changed_pixels ?? image.changedPixels ?? null,
      }]));
      const width = Math.max(0, Number(tile.width || 0));
      const height = Math.max(0, Number(tile.height || 0));
      const y = Number(tile.y ?? nextY);
      nextY = Math.max(nextY, y + height);
      const entry = {
        ...tile, id: String(tile.id ?? `${id}:${tile.tile_index ?? tileIndex}`),
        sheetId: id, sheetName: sheet.name || id, index: tileIndex,
        address: tile.address || tile.range || '',
        x: Number(tile.x || 0), y, width, height, images,
      };
      const classes = profiles.map(profile => images[profile.id]?.pixelClass);
      entry.comparisonComplete = classes.every(value => value !== undefined && value !== null)
        && (!tile.status || ['equal', 'different', 'complete'].includes(tile.status));
      entry.equalAcrossAll = entry.comparisonComplete && classes.every(value => value === classes[0]);
      return entry;
    });
    const rectangles = tiles.flatMap(tile => [tile, ...Object.keys(tile.images).map(profile => profileRectangle(tile, profile))]);
    return { ...sheet, id, name: sheet.name || id, tiles,
      width: Math.max(Number(sheet.width || 0), ...rectangles.map(tile => tile.x + tile.width)),
      height: Math.max(Number(sheet.height || 0), ...rectangles.map(tile => tile.y + tile.height)) };
  });
  return { ...raw, profiles, sheets };
}

export function imageStatus(entry, profileId) {
  const image = entry.images[profileId], original = entry.images.original;
  if (!image?.url) return 'waiting';
  if (!entry.comparisonComplete || image.pixelClass == null || original?.pixelClass == null) return 'received';
  if (profileId === 'original') return entry.comparisonComplete ? (entry.equalAcrossAll ? 'equal' : 'different') : 'received';
  return image.pixelClass === original.pixelClass ? 'equal' : 'different';
}

export function groupedPanels(entry, profiles) {
  const groups = [], byClass = new Map();
  for (const profile of profiles) {
    const image = entry.images[profile.id];
    if (!image?.url) continue;
    // Unclassified images may never be merged merely because both lack a class.
    const pixelClass = entry.comparisonComplete ? image.pixelClass : null;
    const key = pixelClass ?? `pending:${profile.id}`;
    if (profile.id === 'original') groups.unshift({ ids: [profile.id], original: true, pixelClass });
    else {
      if (!byClass.has(key)) { const group = { ids: [], original: false, pixelClass }; byClass.set(key, group); groups.push(group); }
      byClass.get(key).ids.push(profile.id);
    }
  }
  const originalClass = entry.images.original?.pixelClass;
  return groups.sort((a, b) => Number(!a.original) - Number(!b.original)
    || Number(a.pixelClass !== originalClass) - Number(b.pixelClass !== originalClass));
}

export function visibleTiles(sheet, view, limit = 24) {
  if (!sheet) return [];
  const intersects = tile => tile.width > 0 && tile.height > 0
    && tile.x < view.x + view.width && tile.x + tile.width > view.x
    && tile.y < view.y + view.height && tile.y + tile.height > view.y;
  return sheet.tiles.filter(tile => intersects(tile) || Object.keys(tile.images || {}).some(profile => intersects(profileRectangle(tile, profile)))).slice(0, limit);
}

export function profileRectangle(tile, profile) {
  const image = tile.images?.[profile];
  return { x: Number(image?.x ?? tile.x), y: Number(image?.y ?? tile.y),
    width: Number(image?.width ?? tile.width), height: Number(image?.height ?? tile.height) };
}

export function activeGroupCount(queue) {
  if (!queue || queue.running === false || queue.runner_alive === false) return 0;
  if (queue.open_groups != null) return Number(queue.open_groups);
  if (queue.active_groups != null) return Number(queue.active_groups);
  return (queue.groups || []).filter(group => typeof group.active === 'boolean'
    ? group.active
    : ['active', 'opening', 'planning', 'capturing', 'comparing', 'running', 'waiting_cleanup', 'retrying_classic'].includes(group.status)).length;
}

export function clippedTile(tile, view) {
  const x = Math.max(tile.x, view.x), y = Math.max(tile.y, view.y);
  const width = Math.min(tile.x + tile.width, view.x + view.width) - x;
  const height = Math.min(tile.y + tile.height, view.y + view.height) - y;
  return { x, y, width, height,
    u0: (x - tile.x) / tile.width, u1: (x + width - tile.x) / tile.width,
    v0: 1 - (y + height - tile.y) / tile.height, v1: 1 - (y - tile.y) / tile.height };
}
