// Labels refer to retained cell ranges rather than inferred pixel-to-cell geometry.
export function formatRange(sheet, region) { return region.range || region.label || 'Retained capture'; }
