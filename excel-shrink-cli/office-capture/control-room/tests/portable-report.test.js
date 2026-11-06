import assert from 'node:assert/strict';
import { atelierRegions } from '../atelier-model.js';
import { portableImageUrl, portablePageUrl, reportRowMatches } from '../report.js';
import { pixelSummary } from '../pixel-summary.js';

const first = `evidence/${'a'.repeat(64)}.png`;
const second = `evidence/${'b'.repeat(64)}.jpg`;

assert.equal(portableImageUrl(first), first);
assert.equal(portableImageUrl(second), second);
for (const path of [null, '/api/evidence/a.png', '../private.png', 'https://example.test/a.png',
  `evidence/../${'a'.repeat(64)}.png`, `evidence/${'a'.repeat(64)}.png?x=1`, 'evidence/a.png']) {
  assert.equal(portableImageUrl(path), null);
}

// Refactoring safeguard: both transports project exactly the same comparisons.
const report = { comparisons: [{ left: 'OR', right: 'SDR', findings: [
  { sheet: 'Data', cell: 'A1:B2', pixels_differ: true, left_png: first, right_png: second },
  { sheet: 'Data', cell: 'A3:B4', pixels_differ: false, left_png: first, right_png: first },
], charts: [{ sheet: 'Chart', chart: 'Revenue', presence_differs: true, left_image: second }] }] };
const portable = atelierRegions(report, true, portableImageUrl);
const live = atelierRegions(report, true);
assert.deepEqual(portable.map(({ leftUrl, rightUrl, ...row }) => row),
  live.map(({ leftUrl, rightUrl, ...row }) => row));
assert.equal(portable[0].leftUrl, first);
assert.equal(portable[0].rightUrl, second);
assert.equal(portable[1].rightUrl, null);
assert.equal(atelierRegions(report, false, portableImageUrl).length, 3);

assert.equal(portablePageUrl('pairs/one/index.html'), 'pairs/one/index.html');
assert.equal(portablePageUrl('pairs/one two/index.html'), 'pairs/one%20two/index.html');
for (const path of ['../index.html', 'http://example.test/index.html', '/pairs/one/index.html',
  'pairs/one/../index.html', 'pairs/one\\index.html', 'pairs/one/index.html?run=x']) {
  assert.equal(portablePageUrl(path), null);
}
assert.equal(reportRowMatches({ name: 'Book.xlsx', status: 'failed', report: 'pairs/a/report.json' }, 'book', 'unavailable'), true);
assert.equal(reportRowMatches({ name: 'Book.xlsx', status: 'reused', reuse: { parts: ['worksheets'] } }, '', 'reused'), true);
assert.equal(reportRowMatches({ name: 'Book.xlsx', different_regions: 0 }, '', 'differences'), false);
assert.equal(reportRowMatches({ name: 'Book.xlsx', different_regions: 1 }, '', 'differences'), true);
assert.equal(pixelSummary({ changed_pixels: 1, max_channel_delta: 2, total_channel_delta: 3 }),
  '1 changed px · max Δ2/255 · ΣΔ 3');
console.log('PASS portable report paths, shared comparison projection, filters and pixel summary');
