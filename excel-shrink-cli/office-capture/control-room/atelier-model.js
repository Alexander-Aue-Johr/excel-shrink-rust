/** Formula paths explain measured pixel changes in the selected capture. */
export function shouldInspectFormulas(metrics) {
  return Number.isFinite(metrics?.changed_pixels) && metrics.changed_pixels > 0;
}

/** Only retained PNGs inside the study evidence tree are addressable by the atelier. */
export function evidenceImageUrl(path) {
  if (typeof path !== 'string' || !path.startsWith('evidence/')) return null;
  const parts = path.split('/');
  if (!path.endsWith('.png') || parts.some(part => !part || part === '.' || part === '..'
      || /[\\:\u0000-\u001f]/.test(part))) return null;
  return '/api/evidence/' + parts.slice(1).map(encodeURIComponent).join('/');
}

/** Preserve the pair and phase: O versus SD differs from O versus rebuilt SD (SDR). */
export function atelierRegions(report, differencesOnly = true, imageUrl = evidenceImageUrl) {
  const regions = [];
  for (const [pairIndex, pair] of (report.comparisons || []).entries()) {
    const entries = ['findings', 'charts'].flatMap(kind =>
      (pair[kind] || []).map((finding, findingIndex) => ({ finding, key: `${pairIndex}:${kind}:${findingIndex}` })));
    for (const { finding, key } of entries) {
      const different = ['pixels_differ', 'geometry_differs', 'rendering_differs', 'presence_differs']
        .some(flag => finding[flag] === true);
      if (differencesOnly && !different) continue;
      regions.push({
        ...finding, key, different, left: pair.left, right: pair.right,
        label: `${finding.sheet} / ${finding.cell || finding.chart} / ${pair.left} / ${pair.right}`,
        leftUrl: imageUrl(finding.left_png || finding.left_image),
        rightUrl: imageUrl(finding.right_png || finding.right_image),
      });
    }
  }
  return regions;
}
