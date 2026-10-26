// Pure navigation mapping: telemetry gives sheet_index/tile_index/range, while
// the Atelier uses normalized sheet and tile identities.
export function findCaptureTile(group, capture) {
  const details = capture?.details;
  if (!group?.sheets?.length || !details) return null;
  const sheetId = details.sheet_index;
  let sheets = group.sheets;
  if (sheetId != null) {
    sheets = sheets.filter((sheet, index) => String(sheet.index ?? index) === String(sheetId)
      || String(sheet.id) === String(sheetId));
  }
  if (details.sheet_name != null) {
    sheets = sheets.filter(sheet => sheet.name === details.sheet_name);
  }
  // Every reported location must agree. Stale or conflicting identifiers must
  // never fall back to a different worksheet or an unrelated tile.
  let tiles = sheets.flatMap(sheet => sheet.tiles || []).filter(tile => tile.status !== 'omitted');
  const normalizedRange = value => typeof value === 'string' ? value.replace(/\$/g, '').toUpperCase() : null;
  if (details.range != null) {
    const range = normalizedRange(details.range);
    if (!range) return null;
    tiles = tiles.filter(tile => normalizedRange(tile.address || tile.range) === range);
  }
  if (details.tile_index != null) {
    tiles = tiles.filter(tile => String(tile.tile_index ?? tile.index) === String(details.tile_index));
  }
  if (details.range == null && details.tile_index == null) return null;
  return tiles.length === 1 ? tiles[0] : null;
}

export function profileForClient(group, clientId) {
  if (!clientId) return null;
  const matches = (group?.profiles || []).filter(profile => profile.id === clientId || group.aliases?.[profile.id] === clientId);
  return matches[0]?.id || clientId;
}

export function latestTileForProfile(group, profile) {
  if (!group || !profile) return null;
  const candidate = group.sheets.flatMap(sheet => sheet.tiles || [])
    .filter(tile => tile.status !== 'omitted' && tile.images?.[profile]?.url)
    .map(tile => ({ tile, stamp: Number(tile.images[profile]?.updated_at ?? tile.updated_at) }))
    .filter(entry => Number.isFinite(entry.stamp) && entry.stamp > 0)
    .sort((a, b) => b.stamp - a.stamp);
  return candidate[0]?.tile || null;
}
