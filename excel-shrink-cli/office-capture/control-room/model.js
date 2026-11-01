// Pure view model: every rectangle and equality class comes from a real job.
export const PRIMARY_PROFILES = [
  { id: 'original', label: 'Original' },
  { id: 'default', label: 'Default' },
  { id: 'maximum-cleaning', label: 'Maximum cleaning' },
  { id: 'too-aggressive-cleaning', label: 'Too aggressive' },
];

export function groupOutcome(group) {
  const status = group?.status;
  const reason = [group?.outcome_error, group?.skip_reason, group?.error, group?.result_reason]
    .find(value => typeof value === 'string' && value.trim());
  if (['screened', 'screening-differences', 'screening-failed', 'validation-incomplete'].includes(status)) {
    return { kind: ['screening-failed', 'validation-incomplete'].includes(status) ? 'warning' : status === 'screening-differences' ? 'different' : 'complete',
      title: status === 'validation-incomplete' ? 'Full validation incomplete' : status === 'screening-failed' ? 'Aggressive screening incomplete' : status === 'screening-differences' ? 'Differences found during screening' : 'Aggressive screening complete',
      detail: 'Original versus Too Aggressive was prioritized. Default/Maximum may have targeted evidence; their full validation follows after screening the entire selection.' };
  }
  if (status === 'skipped') {
    return { kind: 'warning', title: 'Skipped',
      detail: reason || 'The runner did not provide a skip reason. No visual result is established.' };
  }
  if (group?.analysis_summary && ['screened-equal', 'differences'].includes(status)) {
    const changed = group.analysis_summary.raw_differences;
    const recalculated = group.analysis_summary.changed_after_recalculation;
    return {
      kind: changed ? 'different' : 'equal',
      title: changed ? 'Differences in analyzed ranges' : 'Analyzed ranges identical',
      detail: `Original input state. Omitted ranges remain unchecked; no automatic full-sheet follow-up.${recalculated ? ' Captures changed after recalculation; separate recalculated evidence is retained in the report.' : ''}`,
    };
  }
  if (status === 'crc-identical') {
    return { kind: 'warning', title: 'Archive unchanged after shrink',
      detail: 'Length and CRC-32 matched the shrink input. The original archive was reused; no visual comparison was performed.' };
  }
  if (status === 'workbook-only') {
    return { kind: 'warning', title: 'Only workbook.xml cleanup available',
      detail: 'Worksheet XML was unchanged. Baseline repair and visual comparison were skipped.' };
  }
  if (status === 'paused-for-later') {
    return { kind: 'warning', title: 'Full scan paused for later',
      detail: 'Only selected edge captures were compared. Omitted areas remain unchecked; start the deferred full scan to continue.' };
  }
  if (status === 'failed' || status === 'interrupted') {
    return { kind: 'error', title: status === 'failed' ? 'Validation failed' : 'Interrupted',
      detail: reason || 'Validation did not finish. No visual result is established.' };
  }
  if (status === 'unchanged') {
    if (group.outcome_method === 'package-equivalence') {
      return { kind: 'complete', title: 'Package unchanged',
        detail: 'The runner used package equivalence to finish this file. No captures were performed.' };
    }
    const hasComparedCaptures = group.sheets?.some(sheet => sheet.tiles?.some(tile =>
      tile.comparisonComplete && tile.equalAcrossAll && tile.status !== 'omitted'));
    if (hasComparedCaptures) {
      const blankMargins = group.sheets?.some(sheet => sheet.tiles?.some(tile =>
        tile.comparisonComplete && tile.blank_margin_normalized));
      if (blankMargins) {
        return { kind: 'equal', title: 'No visible content difference',
          detail: 'Compared pixels matched; extra capture margins contained only opaque white. Full images and original dimensions were retained. Omitted areas remain unchecked.' };
      }
      return { kind: 'equal', title: 'Compared captures identical',
        detail: 'The compared captures matched. Any omitted areas remain unchecked.' };
    }
    return { kind: 'complete', title: 'No differences detected',
      detail: 'The recorded result reports no differences. No completed capture evidence is available in this view.' };
  }
  if (status === 'differences') {
    const visualDifference = group.sheets?.some(sheet => sheet.tiles?.some(tile =>
      tile.comparisonComplete && !tile.equalAcrossAll && tile.status !== 'omitted'));
    return { kind: 'different', title: visualDifference ? 'Visual differences found' : 'Differences found',
      detail: visualDifference ? 'Select a differing capture to inspect the profiles side by side.'
        : 'Differences may concern cell values or visual checks. See the report for the recorded findings.' };
  }
  if (['complete', 'completed', 'finished'].includes(status)) {
    return { kind: 'complete', title: 'Processing completed',
      detail: reason || 'See the report for the recorded comparison result.' };
  }
  return null;
}

export function processMeasurements(group, id) {
  const measurements = group?.measurements || {};
  const original = measurements.original || {};
  const measured = measurements[id] || measurements[Object.keys(group?.aliases || {}).find(key => group.aliases[key] === id)] || {};
  const known = value => typeof value === 'number' && Number.isFinite(value) && value >= 0;
  const number = (value, digits = 2) => value.toLocaleString('en-US', {maximumFractionDigits: digits});
  const delta = (value, baseline, unit) => {
    const change = value - baseline;
    return `${change > 0 ? '+' : change < 0 ? '−' : '±'}${number(Math.abs(change))} ${unit}${baseline > 0 ? ` · ${change > 0 ? '+' : change < 0 ? '−' : '±'}${number(Math.abs(change / baseline * 100), 1)} %` : ''} vs original`;
  };
  const size = known(measured.bytes) ? `File: ${number(measured.bytes / 1048576)} MiB${id !== 'original' && known(original.bytes) ? ` (${delta(measured.bytes / 1048576, original.bytes / 1048576, 'MiB')})` : ''}` : 'File size not known yet';
  const startup = measured.startup, baselineStartup = original.startup;
  const native = startup?.method === 'launch-to-addin-ready-v1' && known(startup.seconds);
  const comparableNative = native && baselineStartup?.method === startup.method && known(baselineStartup.seconds);
  const opening = native
    ? `Launch to add-in ready: ${number(startup.seconds)} s${id !== 'original' && comparableNative ? ` (${delta(startup.seconds, baselineStartup.seconds, 's')})` : ''}`
    : known(measured.open_seconds) ? `COM Open (historical): ${number(measured.open_seconds)} s${id !== 'original' && known(original.open_seconds) ? ` (${delta(measured.open_seconds, original.open_seconds, 's')})` : ''}`
      : group?.capture_mode === 'classic' || group?.status === 'retrying_classic' ? 'Startup unavailable (classic capture)' : 'Startup not measured yet';
  const extra = [];
  if (known(measured.shrink_seconds)) extra.push(`Shrink: ${number(measured.shrink_seconds)} s`);
  if (native) {
    for (const [field, label] of [['private_bytes', 'Excel private bytes at ready'], ['working_set_bytes', 'Excel working set at ready']]) {
      if (known(startup[field])) extra.push(`${label}: ${number(startup[field] / 1048576, 1)} MiB${id !== 'original' && comparableNative && known(baselineStartup[field]) ? ` (${delta(startup[field] / 1048576, baselineStartup[field] / 1048576, 'MiB')})` : ''}`);
    }
    if (id !== 'original' && startup.client_id === baselineStartup?.client_id) extra.push('Shared original client; no independent startup');
  } else if (known(measured.memory_bytes)) extra.push(`COM working set at open (historical): ${number(measured.memory_bytes / 1048576, 1)} MiB${id !== 'original' && known(original.memory_bytes) ? ` (${delta(measured.memory_bytes / 1048576, original.memory_bytes / 1048576, 'MiB')})` : ''}`);
  return {size, opening, extra:extra.join(' · ')};
}

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
    let tiles = (sheet.tiles || []).map((tile, tileIndex) => {
      const images = Object.fromEntries(Object.entries(tile.images || {}).map(([profile, image]) => [profile, {
        ...image,
        url: versioned(image.url || image.preview_url || image.previewUrl, image.updated_at ?? tile.updated_at),
        previewUrl: versioned(image.preview_url || image.previewUrl || image.url, image.updated_at ?? tile.updated_at),
        fullUrl: versioned(image.full_url || image.fullUrl || (image.retainedFullResolution ? image.url : null), image.updated_at ?? tile.updated_at),
        pixelClass: image.pixel_class ?? image.pixelClass ?? null,
        maskUrl: versioned(image.mask_url || image.maskUrl, image.updated_at ?? tile.updated_at),
        changedPixels: image.changed_pixels ?? image.changedPixels ?? null,
        blankMarginPixels: image.blank_margin_pixels ?? image.blankMarginPixels ?? 0,
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
      entry.equalAcrossAll = entry.comparisonComplete && (typeof tile.equal_across_all === 'boolean'
        ? tile.equal_across_all : classes.every(value => value === classes[0]));
      return entry;
    });
    const sourceHeight = Math.max(Number(sheet.height || 0), nextY);
    tiles = compactOmissions(sheet, tiles, profiles);
    const rectangles = tiles.flatMap(tile => [tile, ...new Set([...Object.keys(tile.images), ...Object.keys(tile.planned_images || {})])].map(value => typeof value === 'string' ? profileRectangle(tile, value) : value));
    return { ...sheet, id, name: sheet.name || id, tiles,
      source_height: sourceHeight,
      width: Math.max(Number(sheet.width || 0), ...rectangles.map(tile => tile.x + tile.width)),
      height: Math.max(tiles.some(tile => tile.status === 'omitted') ? 0 : Number(sheet.height || 0), ...rectangles.map(tile => tile.y + tile.height), 0) };
  });
  return { ...raw, profiles, sheets };
}

export function imageStatus(entry, profileId) {
  if (entry.status === 'omitted') return 'omitted';
  const image = entry.images[profileId], original = entry.images.original;
  if (!image?.url) {
    return entry.comparison_method === 'png-sha256' && entry.equalAcrossAll
      ? 'hash-equal' : 'waiting';
  }
  if (!entry.comparisonComplete || image.pixelClass == null || original?.pixelClass == null) return 'received';
  if (profileId === 'original') return entry.comparisonComplete ? (entry.equalAcrossAll ? 'equal' : 'different') : 'received';
  return image.pixelClass === original.pixelClass || (entry.blank_margin_normalized && image.blankMarginPixels > 0 && image.changedPixels === 0)
    ? 'equal' : 'different';
}

export function tileEvidenceDescription(tile) {
  if (tile.status === 'omitted') return omissionDescription(tile);
  const details = [tile.address];
  if (tile.capture_budget_fallback === 'single-visible-row') {
    const pixels = Number(tile.capture_pixels);
    const size = Number.isFinite(pixels) ? `${(pixels / 1000000).toFixed(1)} MP` : 'a larger image';
    details.push(`This indivisible row and its border rows require ${size}, above the target tile size. Single-row fallback limit: 32 MP.`);
  }
  if (tile.comparison_method === 'png-sha256' && tile.equalAcrossAll) {
    details.push('Identical PNG SHA-256 across all profiles; captured images matched.');
    if (!Object.values(tile.images).some(image => image.url)) {
      details.push('No image preview retained. This capture was checked, not omitted.');
    }
  }
  if (tile.blank_margin_normalized) {
    details.push('At least one profile has a verified opaque-white margin; full images and original dimensions were retained.');
  }
  if (tile.context_for?.length) {
    details.push(`Full-resolution context beside ${tile.context_for.length} differing capture${tile.context_for.length === 1 ? '' : 's'}.`);
  }
  return details.join(' \u00b7 ');
}

export function captureTransportSummary(controls, group) {
  const labels = { 'live-images': 'Live images', 'hash-first': 'Hash first' };
  const requested = controls.capture_transport || 'live-images';
  const active = group?.capture_pipeline?.transport;
  if (!active) return `${labels[requested]} selected for the next primary capture phase.`;
  return active === requested
    ? `Selected group: ${labels[active] || active}.`
    : `Selected group: ${labels[active] || active}. Next primary capture phase: ${labels[requested]}.`;
}

// Collapse only explicit omissions. Pending captures retain their real geometry.
// Image dimensions and tile-local inspection coordinates never change; only the
// display's cumulative y offsets are shortened, separately for each profile.
function compactOmissions(sheet, tiles, profiles) {
  const runs = [];
  for (const tile of tiles) {
    if (tile.status !== 'omitted') continue;
    const previous = runs.at(-1);
    if (previous && previous.at(-1).index + 1 === tile.index
      && previous.at(-1).core_rows?.[1] === tile.core_rows?.[0]) previous.push(tile);
    else runs.push([tile]);
  }
  if (!runs.length) return tiles;
  const ids = [...new Set(['original', ...profiles.map(profile => profile.id)])];
  const cuts = Object.fromEntries(ids.map(profile => [profile, runs.map(run => {
    const rects = run.map(tile => profileRectangle(tile, profile));
    const start = Math.min(...rects.map(rect => rect.y));
    const end = Math.max(...rects.map(rect => rect.y + rect.height));
    const width = Math.max(...rects.map(rect => rect.width));
    return { start, end, height: Math.min(end - start, Math.max(64, Math.min(2048, width * .12))) };
  })]));
  const displayY = (y, profile) => y - cuts[profile].reduce((sum, cut) => {
    const progress = Math.max(0, Math.min(cut.end - cut.start, y - cut.start));
    return sum + (cut.end > cut.start ? progress * (1 - cut.height / (cut.end - cut.start)) : 0);
  }, 0);
  const first = new Map(runs.map((run, index) => [run[0].id, { run, index }]));
  const result = [];
  for (const tile of tiles) {
    if (tile.status === 'omitted') {
      const info = first.get(tile.id); if (!info) continue;
      const { run, index } = info, last = run.at(-1);
      const related = (sheet.capture_coverage?.omitted_ranges || []).filter(range =>
        range.core_rows?.[0] < last.core_rows?.[1] && range.core_rows?.[1] > tile.core_rows?.[0]);
      const gap = { ...tile, id: `omitted:${tile.id}`, source_y: tile.y,
        y: displayY(tile.y, 'original'), height: cuts.original[index].height,
        source_height: cuts.original[index].end - cuts.original[index].start,
        core_rows: [tile.core_rows?.[0], last.core_rows?.[1]], omitted_count: run.length,
        omission_reasons: [...new Set(related.flatMap(range => range.reasons || []))],
        comparisonComplete: false, equalAcrossAll: false, images: {},
        planned_images: Object.fromEntries(ids.map(profile => [profile, {
          ...profileRectangle(tile, profile), y: displayY(cuts[profile][index].start, profile), height: cuts[profile][index].height,
        }])) };
      gap.address = `Rows ${Number(gap.core_rows[0]) + 1}\u2013${gap.core_rows[1]}`;
      result.push(gap);
    } else {
      const move = (values = {}) => Object.fromEntries(Object.entries(values).map(([profile, image]) => [profile, {
        ...image, source_y: image.y ?? tile.y, y: displayY(Number(image.y ?? tile.y), cuts[profile] ? profile : 'original'),
      }]));
      result.push({ ...tile, source_y: tile.y, y: displayY(tile.y, 'original'),
        images: move(tile.images), planned_images: move(tile.planned_images) });
    }
  }
  return result;
}

export function omissionDescription(tile) {
  return [`${tile.omitted_count || 1} capture rows omitted`, tile.address,
    'Not visually checked; shown in a compact studio view.',
    ...(tile.omission_reasons || [])].filter(Boolean).join(' \u00b7 ');
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
  const image = tile.images?.[profile] || tile.planned_images?.[profile];
  return { x: Number(image?.x ?? tile.x), y: Number(image?.y ?? tile.y),
    width: Number(image?.width ?? tile.width), height: Number(image?.height ?? tile.height) };
}

export function activeGroupCount(queue) {
  if (!queue || queue.running === false || queue.runner_alive === false) return 0;
  if (queue.open_groups != null) return Number(queue.open_groups);
  if (queue.active_groups != null) return Number(queue.active_groups);
  return (queue.groups || []).filter(group => typeof group.active === 'boolean'
    ? group.active
    : ['active', 'opening', 'planning', 'capturing', 'comparing', 'running', 'waiting_cleanup', 'cleanup_blocked', 'retrying_classic'].includes(group.status)).length;
}

export function clippedTile(tile, view) {
  const x = Math.max(tile.x, view.x), y = Math.max(tile.y, view.y);
  const width = Math.min(tile.x + tile.width, view.x + view.width) - x;
  const height = Math.min(tile.y + tile.height, view.y + view.height) - y;
  return { x, y, width, height,
    u0: (x - tile.x) / tile.width, u1: (x + width - tile.x) / tile.width,
    v0: 1 - (y + height - tile.y) / tile.height, v1: 1 - (y - tile.y) / tile.height };
}

// Cards share a 16:9 shape. A near-square card grid keeps the complete wall
// close to 16:9 as well, without changing the worksheet's pixel coordinates.
export function sheetLayout(sheets) {
  const columns = Math.max(1, Math.ceil(Math.sqrt(sheets.length)));
  const rows = Math.max(1, Math.ceil(sheets.length / columns));
  const width = 16, height = 9, gap = .65;
  return { columns, rows, width: columns * (width + gap) - gap, height: rows * (height + gap) - gap,
    cards: sheets.map((sheet, index) => ({ id: sheet.id, width, height,
      x: (index % columns - (columns - 1) / 2) * (width + gap),
      y: ((rows - 1) / 2 - Math.floor(index / columns)) * (height + gap) })) };
}

export function fitSheetView(sheet, aspect) {
  const width = Math.max(1, sheet?.width || 1, (sheet?.height || 1) * aspect);
  return { x: 0, y: 0, width, height: width / aspect };
}

export function inspectionRegions(tile, onlyDifferences = false) {
  if (!tile || tile.status === 'omitted') return [];
  if (tile.inspection_regions?.length) return tile.inspection_regions
    .filter(region => !onlyDifferences || Number(region.changed_pixels) > 0)
    .map(region => ({ ...region, id: String(region.id), exact: true }));
  // Old results retain their original evidence. A display grid helps inspect
  // huge images, but cannot claim to know which subregion contains a change.
  const columns = Math.max(1, Math.ceil(tile.width / 1024)), rows = Math.max(1, Math.ceil(tile.height / 576));
  const width = tile.width / columns, height = tile.height / rows;
  return Array.from({ length: columns * rows }, (_, index) => ({ id: `display-${index}`, x: index % columns * width,
    y: Math.floor(index / columns) * height, width, height, exact: false }));
}

export function inspectionEntry(tile, region, profiles) {
  if (!region) return tile;
  // Whole-capture equality also proves equality for any common crop. A
  // difference elsewhere in a capture never proves a difference here.
  const classes = tile.equalAcrossAll ? Object.fromEntries(profiles.map(profile => [profile.id, tile.images[profile.id]?.pixelClass])) : region.pixel_classes || {};
  const complete = (region.exact || tile.equalAcrossAll) && profiles.every(profile => classes[profile.id] != null);
  return { ...tile, inspection: region, comparisonComplete: complete,
    equalAcrossAll: complete && (tile.equalAcrossAll || profiles.every(profile => classes[profile.id] === classes.original)),
    images: Object.fromEntries(Object.entries(tile.images).map(([profile, image]) => [profile, { ...image,
      pixelClass: complete ? classes[profile] : null,
      changedPixels: region.changed_pixels_by_profile?.[profile] ?? image.changedPixels }])) };
}

export function differenceTargets(tiles) {
  return tiles.filter(tile => tile.comparisonComplete && !tile.equalAcrossAll)
    .flatMap(tile => inspectionRegions(tile, true).map(region => ({ tile, region })));
}

export function heartbeatAge(client, now = Date.now()) {
  const raw = client.last_heartbeat_unix_ms ?? client.last_heartbeat_at ?? client.last_heartbeat;
  const time = typeof raw === 'number' ? raw : Date.parse(raw);
  return Number.isFinite(time) ? Math.max(0, now - time) : null;
}
