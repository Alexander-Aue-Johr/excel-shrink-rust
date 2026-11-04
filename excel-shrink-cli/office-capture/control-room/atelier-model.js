/** Only retained PNGs inside the study evidence tree are addressable by the atelier. */
export function evidenceImageUrl(path) {
  if (typeof path !== 'string' || !path.startsWith('evidence/')) return null;
  const parts = path.split('/');
  if (!path.endsWith('.png') || parts.some(part => !part || part === '.' || part === '..'
      || /[\\:\u0000-\u001f]/.test(part))) return null;
  return '/api/evidence/' + parts.slice(1).map(encodeURIComponent).join('/');
}

/** Preserve the pair and phase: O versus SD differs from O versus rebuilt SD (SDR). */
export function atelierRegions(report, differencesOnly = true) {
  const regions = [];
  for (const pair of report.comparisons || []) {
    for (const finding of [...(pair.findings || []), ...(pair.charts || [])]) {
      const different = ['pixels_differ', 'geometry_differs', 'rendering_differs', 'presence_differs']
        .some(flag => finding[flag] === true);
      if (differencesOnly && !different) continue;
      regions.push({
        ...finding, different, left: pair.left, right: pair.right,
        label: `${finding.sheet} / ${finding.cell || finding.chart} / ${pair.left} / ${pair.right}`,
        leftUrl: evidenceImageUrl(finding.left_png || finding.left_image),
        rightUrl: evidenceImageUrl(finding.right_png || finding.right_image),
      });
    }
  }
  return regions;
}
