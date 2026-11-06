const number = value => Number(value || 0).toLocaleString('en-US');

/** Descriptive measurements, never a severity threshold that certifies a pass. */
export function pixelSummary(metrics) {
  if (!metrics) return 'Pixel measurements pending';
  const text = `${number(metrics.changed_pixels)} changed px · max Δ${number(metrics.max_channel_delta)}/255 · ΣΔ ${number(metrics.total_channel_delta)}`;
  const exterior = metrics.unmatched_pixels
    ? ` · ${number(metrics.unmatched_pixels)} unpaired px (${number(metrics.unmatched_nonwhite_pixels)} non-white)` : '';
  const missing = metrics.unmeasured_regions ? ` · ${number(metrics.unmeasured_regions)} regions unavailable` : '';
  return text + exterior + missing;
}
