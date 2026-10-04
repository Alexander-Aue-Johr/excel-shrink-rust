'use strict';
// Pure algorithm and mocked integration tests. These do NOT simulate Excel's real renderer.
const assert = require('node:assert/strict');
const path = require('node:path');
const fs = require('node:fs');
const vm = require('node:vm');
const corePath = path.resolve(__dirname, '../../_shared/office-addin/web/render-diagnostics.js');
const D = require(corePath);
let tests = 0;
async function test(name, fn) { await fn(); tests++; console.log('PASS', name); }
function header(width, height) {
  const bytes = Buffer.alloc(57);
  Buffer.from('\x89PNG\r\n\x1a\n', 'latin1').copy(bytes);
  bytes.writeUInt32BE(13, 8); bytes.write('IHDR', 12); bytes.writeUInt32BE(width, 16); bytes.writeUInt32BE(height, 20);
  bytes[24] = 8; bytes[25] = 6;
  // Header-only mock has dummy IDAT/CRC. Tested parser intentionally does not pixel-decode it.
  bytes.write('IDAT', 37);
  Buffer.from('0000000049454e44ae426082', 'hex').copy(bytes, 45);
  return bytes.toString('base64');
}
function simulatedSession(cap, change) {
  let seq = 0;
  return { reliable: async (spec) => {
    const n = spec.search_index;
    const p = { ...spec, success: true, sequence: ++seq, range_width_points: 300,
      range_height_points: n * 12.75, png_width: 401, png_height: Math.min(cap, n * 17 + 1) };
    return change ? change(p) : p;
  } };
}
const options = { sheet: ' KHV_2022', axis: 'y', maxIndex: 2791, baseline: [1, 40, 100],
  address: n => `A1:FX${n}`, controlAddress: 'A1:FX40', neighborhoodRadius: 5 };
(async () => {
  await test('PNG dimensions above 16-bit range are not rejected by the file-format parser', () => {
    const p = D.pngHeader(header(15440, 47462)); assert.equal(p.valid_header, true); assert.equal(p.png_height, 47462);
  });
  await test('invalid and empty image payloads', () => {
    for (const p of ['', null, 'not png', header(0, 20), header(2, 2).slice(0, -8)]) assert.equal(D.pngHeader(p).valid_header, false);
  });
  await test('scale and one-pixel offset are fitted separately', () => {
    const m = D.fit([[12.75, 18], [510, 681], [1275, 1701]]);
    assert.ok(Math.abs(m.slope - 4 / 3) < 1e-12); assert.ok(Math.abs(m.intercept - 1) < 1e-10);
  });
  await test('real plateau predicate reaches adjacent boundary, not full-range-succeeded', async () => {
    const r = await D.plateauSearch(simulatedSession(32766), options);
    assert.equal(r.conclusion, 'plateau-boundary-locally-confirmed'); assert.equal(r.first_plateau_index, 1928);
    assert.equal(r.last_below_plateau_index, 1927); assert.equal(r.neighborhood.length, 11);
  });
  await test('plateau value is learned, not hard-coded to 32766', async () => {
    const r = await D.plateauSearch(simulatedSession(20000), options);
    assert.equal(r.plateau_px, 20000); assert.equal(r.first_plateau_index, Math.ceil((20000 - 1) / 17));
  });
  await test('no plateau is reported when full extent has native dimensions', async () => {
    const r = await D.plateauSearch(simulatedSession(Infinity), options);
    assert.equal(r.conclusion, 'no-saturation-observed-up-to-test-limit'); assert.equal(r.plateau_px, null);
  });
  await test('Office failure is unknown, never a plateau branch', async () => {
    const r = await D.plateauSearch(simulatedSession(32766, p => p.role === 'plateau-binary-search' ? null : p), options);
    assert.equal(r.conclusion, 'inconclusive'); assert.equal(r.first_plateau_index, null);
  });
  await test('unstable repeated results are inconclusive', async () => {
    const r = await D.plateauSearch(simulatedSession(32766, p => p.role === 'plateau-boundary-repeat' ? { ...p, png_height: p.png_height - 10 } : p), options);
    assert.equal(r.conclusion, 'inconclusive');
  });
  await test('clipping and scaling interpretations use marker positions, not just PNG size', () => {
    const markers = [10, 100, 200, 400, 600, 800].map((x, i) => ({ name: 'm' + i, start_points: x, end_points: x + 4 }));
    function probe(scaled) {
      return { success: true, png_height: 500, range_height_points: 1000, marker_analysis: { markers: markers.map(m => {
        const a = scaled ? (m.start_points + 2) / 1000 * 499 + .5 : m.start_points + 2;
        return { name: m.name, count: a < 500 ? 4 : 0, first: a - 2, last: a + 2, segments: a < 500 ? [[a - 2, a + 2]] : [] };
      }) } };
    }
    assert.equal(D.markerVerdict(probe(false), markers, 'y', { slope: 1, intercept: 0 }).conclusion, 'consistent-with-clipping');
    assert.equal(D.markerVerdict(probe(true), markers, 'y', { slope: 1, intercept: 0 }).conclusion, 'consistent-with-scaling-on-tested-axis');
  });
  await test('mocked height-mode integration: correct workbook, token, progress and result', async () => {
    const realSetTimeout = global.setTimeout;
    global.setTimeout = (fn, ms) => realSetTimeout(fn, ms === 100 ? 0 : ms);
    const posts = [];
    global.ExperimentUi = Object.fromEntries(['setPhase', 'log', 'setWorkbook', 'updateProgress', 'fail'].map(k => [k, () => {}]));
    global.Office = { context: { diagnostics: { host: 'Excel', platform: 'mock', version: 'mock' } } };
    global.Excel = { run: async fn => {
      const c = { trace: () => {}, sync: async () => {}, workbook: { name: 'krankenhausverzeichnis-3500100227005.xlsx', load: () => {}, worksheets: { getItem: name => {
        assert.equal(name, ' KHV_2022');
        return { getRange: address => {
          const n = Number(address.match(/\d+$/)[0]);
          return { address, width: 300, height: n * 12.75, left: 0, top: 0, rowCount: n, columnCount: 180,
            load: () => {}, getImage: () => ({ value: header(401, Math.min(32766, n * 17 + 1)) }) };
        } };
      } } } };
      return fn(c);
    } };
    global.fetch = async (url, init) => {
      if (url === '/config') return { ok: true, json: async () => ({ experiment: 'range-image-height-limit', diagnostic_run_id: 'test-run', diagnostic_options: {} }) };
      assert.equal(init.headers['X-VV-Run-Id'], 'test-run');
      posts.push([url, JSON.parse(init.body)]);
      return { ok: true, json: async () => url === '/diagnostic-claim' ? { acquired: true } : { ok: true } };
    };
    try {
      const file = path.join(path.dirname(corePath), 'modes/range-image-height-limit.js');
      await vm.runInThisContext(fs.readFileSync(file, 'utf8'), { filename: file });
      const result = posts.find(([url]) => url === '/result')?.[1];
      assert.equal(result.status, 'complete'); assert.equal(result.analysis.height_plateau.first_plateau_index, 1928);
      assert.ok(posts.filter(([url]) => url === '/diagnostic-progress').length >= 20);
      assert.equal(posts.some(([url]) => url === '/image-batch'), false);
    } finally { global.setTimeout = realSetTimeout; }
  });

  for (const kind of ['units', 'markers']) {
    await test(`mocked ${kind} mode: metadata, API calls and diagnostic results`, async () => {
      const realSetTimeout = global.setTimeout;
      global.setTimeout = (fn, ms) => realSetTimeout(fn, ms === 100 ? 0 : ms);
      const manifest = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../fixtures/manifest.json'), 'utf8'));
      const f = manifest.fixtures.find(x => x.kind === kind);
      const posts = [], images = new Map();
      function number(s) { return [...s].reduce((n, c) => n * 26 + c.charCodeAt(0) - 64, 0); }
      function geometry(sheet, address) {
        const [, ac, ar, bc, br] = address.match(/^([A-Z]+)(\d+):([A-Z]+)(\d+)$/);
        const c1 = number(ac), c2 = number(bc), r1 = +ar, r2 = +br;
        const pxWidth = i => {
          if (sheet === 'Hidden' && [3, 4].includes(i)) return 0;
          const w = ['Units', 'Zoom150', 'GridOn', 'Borders'].includes(sheet) ? f.widths[i - 1] : ['Fractional', 'Defaults'].includes(sheet) ? 8.43 : 10;
          return Math.floor(((256 * w + Math.floor(128 / 7)) / 256) * 7);
        };
        const ptHeight = i => sheet === 'Hidden' && [3, 4].includes(i) ? 0 : ['Units', 'Zoom150', 'GridOn', 'Borders'].includes(sheet) ? f.heights[i - 1] : ['Fractional', 'Defaults'].includes(sheet) ? 12.6 : 12.75;
        function sum(fn, lo, hi) { let n = 0; for (let i = lo; i <= hi; i++) n += fn(i); return n; }
        return { address: `'${sheet}'!${address}`, width: sum(pxWidth, c1, c2) * .75,
          height: sum(ptHeight, r1, r2), top: sum(ptHeight, 1, r1 - 1), left: sum(pxWidth, 1, c1 - 1) * .75,
          rowCount: r2 - r1 + 1, columnCount: c2 - c1 + 1 };
      }
      global.Excel = { run: async fn => fn({ trace() {}, async sync() {}, workbook: { name: f.workbook, load() {}, worksheets: { getItem: sheet => ({ activate() {}, getRange: address => {
        // The font query uses a single cell rather than an address pair.
        if (address === 'A1') return { format: { font: { name: f.normal_font, size: f.normal_font_size_pt, load() {} } } };
        const g = geometry(sheet, address);
        return { ...g, load() {}, getImage: () => ({ value: header(Math.min(32766, Math.round(g.width * 4 / 3) + 1), Math.min(32766, Math.round(g.height * 4 / 3) + 1)) }) };
      } }) } } }) };
      global.fetch = async (url, init) => {
        if (url === '/config') return { ok: true, json: async () => ({ experiment: 'range-image-calibration', diagnostic_run_id: 'mock-synthetic', diagnostic_options: { fixtures: [f] } }) };
        assert.equal(init.headers['X-VV-Run-Id'], 'mock-synthetic');
        const b = JSON.parse(init.body); posts.push([url, b]); let value = { ok: true };
        if (url === '/diagnostic-claim') value = { acquired: true };
        if (url === '/image-batch') {
          value = { captures: b.images.map(i => {
            images.set(i.id, { ...i, ...D.pngHeader(i.base64) });
            return { id: i.id, relative_png: `${f.workbook}/${i.id}.png` };
          }) };
        }
        if (url === '/diagnostic-markers') {
          const image = images.get(b.id), extent = b.axis === 'y' ? image.png_height : image.png_width;
          value = { markers: b.markers.map(m => {
            const first = Math.round(m.start_points * 4 / 3), last = Math.min(extent - 1, Math.round(m.end_points * 4 / 3) - 1);
            return { name: m.name, first: first <= last ? first : null, last: first <= last ? last : null,
              count: Math.max(0, last - first + 1), segments: first <= last ? [[first, last]] : [] };
          }) };
        }
        return { ok: true, json: async () => value };
      };
      try {
        const file = path.join(path.dirname(corePath), 'modes/range-image-calibration.js');
        await vm.runInThisContext(fs.readFileSync(file, 'utf8'), { filename: file });
        const result = posts.find(([url]) => url === '/result')?.[1];
        assert.equal(result.status, 'complete'); assert.ok(result.probe_count > 40);
        if (kind === 'units') {
          assert.ok(Math.abs(result.analysis.unit_conversion.png_x_from_office_points.slope - 4 / 3) < .001);
          assert.equal(result.analysis.view_controls.length, 8);
          assert.equal(result.analysis.hidden_geometry.row_count, 8);
        } else {
          assert.equal(result.analysis.synthetic_height_plateau.plateau_px, 32766);
          assert.equal(result.analysis.synthetic_width_plateau.plateau_px, 32766);
          assert.equal(result.analysis.vertical_marker_control_ok, true);
          assert.ok(result.analysis.vertical_marker_evidence.every(e => e.verdict.conclusion === 'consistent-with-clipping'));
          assert.equal(result.analysis.horizontal_marker_evidence.conclusion, 'consistent-with-clipping');
        }
      } finally { global.setTimeout = realSetTimeout; }
    });
  }
  console.log(`${tests} tests passed. Real Excel rendering, Windows PowerShell and .NET integration require Windows.`);
})().catch(e => { console.error(e); process.exitCode = 1; });
