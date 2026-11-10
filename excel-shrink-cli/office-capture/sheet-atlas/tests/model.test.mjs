import { test } from 'node:test';
import assert from 'node:assert/strict';
import { gzipSync } from 'node:zlib';
import { readGzip, contextKey, assetUrl, candidatePair, sheetSummary, recordedOutcome, groupOutcome } from '../model.js';
import { bookLayout } from '../gallery.js';
import { nativeZoomRegion } from '../comparison.js';

test('static gzip transport accepts compressed bytes and host-decoded JSON', async () => {
  const savedFetch = globalThis.fetch;
  try {
    for (const bytes of [gzipSync('{"schema":1}'), Buffer.from('{"schema":1}')]) {
      globalThis.fetch = async () => new Response(bytes);
      assert.deepEqual(await readGzip('atlas-data/catalog.json.gz'), { schema: 1 });
    }
    globalThis.fetch = async () => new Response('missing', { status: 404 });
    await assert.rejects(readGzip('atlas-data/missing.json.gz'), /404/);
  } finally { globalThis.fetch = savedFetch; }
});

test('independent capture contexts require matching original pixels, range and states', () => {
  const capture = { range: 'A1:C9', state: 'OR → SDR', original: 'https://example.test/a/hash.png',
    finding: { left_geometry: { left: 0, top: 0, width: 90, height: 100 } }, signature: { policy: 'v11' } };
  assert.equal(contextKey(capture), contextKey({ ...capture, original: 'https://example.test/b/hash.png' }));
  assert.notEqual(contextKey(capture), contextKey({ ...capture, range: 'B1:D9' }));
  assert.notEqual(contextKey(capture), contextKey({ ...capture, state: 'O → SD' }));
  assert.notEqual(contextKey(capture), contextKey({ ...capture, original: 'https://example.test/a/other.png' }));
  assert.notEqual(contextKey(capture), contextKey({ ...capture, signature: { policy: 'v12' } }));
  assert.equal(candidatePair({ left: 'O', right: 'OR' }), false);
});

test('layout retains all sheet slots, including workbooks without findings', () => {
  const books = [{ id: 'one', sheets: [{ id: 1 }, { id: 2 }] }, { id: 'two', sheets: Array.from({ length: 215 }, (_, id) => ({ id })) }];
  const layout = bookLayout(books);
  assert.equal(layout.layouts.length, 2);
  assert.equal(layout.layouts[0].book.sheets.length, 2);
  assert.equal(layout.layouts[1].book.sheets.length, 215);
  assert.ok(layout.layouts[1].rows * layout.layouts[1].sheetColumns >= 215);
  assert.ok(Number.isFinite(layout.width) && layout.width > 0);
});

test('static report paths cannot escape the exported directory', () => {
  assert.throws(() => assetUrl('https://outside.invalid/report.json.gz'), /escapes/);
  assert.throws(() => assetUrl('../outside.json.gz'), /escapes/);
});

test('report stages use square workbook grids from three through seven columns', () => {
  for (const side of [3, 4, 5, 6, 7]) {
    const books = Array.from({ length: side * side }, (_, id) => ({
      id,
      sheets: [{ name: 'Sheet1' }],
    }));
    const { layouts } = bookLayout(books);
    assert.equal(layouts.length, side * side);
    assert.equal(new Set(layouts.map(item => item.x)).size, side);
    assert.equal(new Set(layouts.map(item => item.y)).size, side);
    assert.equal(new Set(layouts.map(item => `${item.x},${item.y}`)).size, side * side);
  }
});

test('worksheet names cannot resolve inherited JavaScript object properties', () => {
  assert.equal(sheetSummary({ sheets: {} }, 'constructor'), undefined);
  const sheets = JSON.parse('{"__proto__":{"differences":2}}');
  assert.equal(sheetSummary({ sheets }, '__proto__').differences, 2);
});

test('native magnifiers contain a changed pixel when separated changes surround an empty center', () => {
  const mask = new Uint8Array(1000 * 700);
  mask[600 * 1000 + 100] = 1; mask[650 * 1000 + 900] = 1;
  const region = nativeZoomRegion(mask, 1000, 700);
  assert.ok(region.x <= 100 && region.x + region.w > 100);
  assert.ok(region.y <= 600 && region.y + region.h > 600);
  assert.equal(region.h, 120);
  assert.deepEqual(nativeZoomRegion(new Uint8Array(4), 2, 2), { x: 0, y: 0, w: 2, h: 2 });
});

test('comparison header colors follow recorded verdicts and preserve unknown outcomes', () => {
  assert.equal(recordedOutcome({ pixels_compared: true }), 'equal');
  assert.equal(recordedOutcome({ pixels_compared: true, geometry_differs: true }), 'different');
  assert.equal(recordedOutcome({ capture_extent_ignored: true, pixels_compared: true }), 'equal');
  assert.equal(recordedOutcome({}), 'unknown');
  assert.equal(groupOutcome(['original', 'same'], { original: 'equal', same: 'equal' }), 'equal');
  assert.equal(groupOutcome(['original', 'changed'], { original: 'equal', changed: 'different' }), 'different');
  assert.equal(groupOutcome(['original', 'missing'], { original: 'equal' }), 'unknown');
});
