const test = require('node:test');
const assert = require('node:assert/strict');
const {createWorker, selectedTiles, encoded} = require('../web/resident.js');

function fixture(workbookName = 'fixture.xlsx') {
  const captures = [], uploads = [], heightSheets = [], geometryReads = [];
  let geometriesCreated = 0;
  let content = 'stable';
  const encode = (width, height) => Buffer.from(JSON.stringify({width, height, content})).toString('base64');
  globalThis.CaptureGeometry = {
    address: rect => `${rect.r0}:${rect.r1}`,
    pngSize: value => JSON.parse(Buffer.from(value, 'base64').toString()),
    probeEnd: () => 3,
    SheetGeometry: class {
      constructor() {geometriesCreated++; this.rows = Array(10).fill(1); this.maxPixels = 32000000; this.fixedWidth = null;}
      async read(maximum) {geometryReads.push(maximum);}
      async calibrate() {}
      imageWidth() {return 3;}
      tiles(end, targets, cuts = [[0, 3], [3, 6], [6, 9], [9, 10]]) {
        assert.equal(cuts[0][0], 0);
        assert.equal(cuts.at(-1)[1], 10);
        return cuts.map(([first, last], index) => {
          if (index) assert.equal(cuts[index - 1][1], first);
          assert.ok(last - first <= 3);
          const core = {r0: first, r1: last, c0: 0, c1: end + 1};
          return {core, raw: core, width: 3, expectedHeight: last - first, cropHeight: last - first, cropY: 0};
        });
      }
    }
  };
  globalThis.CaptureOverflow = {async crop(base64) {return base64;}, async check() {return {rightEdge: false};}};
  const helpers = {
    async office(action) {return action({workbook: {name: workbookName, load() {}, worksheets: {getItem() {return {
      getRange(range) {return {getImage() {
        captures.push(range);
        const [first, last] = range.split(':').map(Number);
        return {value: encode(3, last - first)};
      }};}
    };}}}, async sync() {}});},
    bounds: sheet => ({bottom: sheet.bottom_row, right: 1}),
    async checkedWorksheet() {}, async hideGridlines() {}, async verifyDrawingBounds() {},
    async rowHeights(sheet) {heightSheets.push(sheet); return {sheet_index: sheet.index, runs: [[0, 10, 1]]};},
    phase() {}, progress() {}
  };
  const worker = createWorker(helpers, {async image(command, metadata, bytes) {
    uploads.push({command, metadata, bytes});
    return {image_path: `images/${command}/${metadata.tile_index}.png`, width: metadata.width,
      height: metadata.height, sha256: 'a'.repeat(64)};
  }}, 'fixture.xlsx');
  const sheet = {index: 1, name: 'Sheet1', bottom_row: 9, row_height_bottom_row: 2,
    horizontal_overflow_required: false, drawing_fallback_required: false};
  return {worker, sheet, captures, uploads, heightSheets, geometryReads, change() {content = 'changed';},
    get geometriesCreated() {return geometriesCreated;}};
}

test('initial resident capture uploads only hashes and preserves full height coverage', async () => {
  const f = fixture();
  const result = await f.worker.execute({id: 'first', kind: 'capture', sheets: [f.sheet]});
  assert.equal(result.tiles.length, 4);
  assert.equal(f.uploads.length, 0);
  assert.equal(result.uploaded_images, 0);
  assert.equal(result.checksum_tiles, 4);
  assert.equal(f.heightSheets[0].bottom_row, 9);
  for (const tile of result.tiles) {
    assert.match(tile.sha256, /^[a-f0-9]{64}$/);
    assert.equal(tile.base64, undefined);
    assert.equal(tile.bytes, undefined);
  }
});

test('a task pane in a different workbook fails before reading sheets or capturing images', async () => {
  const f = fixture('wrong-workbook.xlsx');
  await assert.rejects(f.worker.execute({id: 'first', kind: 'capture', sheets: [f.sheet]}),
    error => error.code === 'WORKBOOK_IDENTITY');
  assert.deepEqual(f.heightSheets, []);
  assert.deepEqual(f.captures, []);
  assert.deepEqual(f.uploads, []);
});

test('a partial whitespace capture takes images only for the requested spans', async () => {
  const f = fixture();
  const result = await f.worker.execute({id: 'partial', kind: 'capture',
    sheets: [{...f.sheet, tile_rows: [[3, 6], [9, 10]]}]});
  assert.deepEqual(f.captures, ['3:6', '9:10']);
  assert.deepEqual(result.sheets[0].tile_rows, [[3, 6], [9, 10]]);
  assert.equal(result.tiles.length, 2);
});

test('the maximum read boundary does not widen initial captures beyond the local overflow probe', async () => {
  const f = fixture();
  const result = await f.worker.execute({id: 'local', kind: 'capture',
    sheets: [{...f.sheet, horizontal_overflow_required: true, capture_right_column: 10}]});
  assert.deepEqual(f.geometryReads, [10]);
  assert.equal(result.sheets[0].capture_right_column, 3);
  assert.ok(result.tiles.every(tile => tile.capture_right_column === 3));
});

test('negotiated capture ends may widen the local probe within the maximum boundary', async () => {
  const f = fixture();
  const sheet = {...f.sheet, horizontal_overflow_required: true, capture_right_column: 10};
  const result = await f.worker.execute({id: 'shared', kind: 'capture', sheets: [{...sheet, capture_end_column: 6}]});
  assert.equal(result.sheets[0].capture_right_column, 6);
  assert.ok(result.tiles.every(tile => tile.capture_right_column === 6));
  for (const [index, end] of [2, 11, 3.5].entries()) {
    await assert.rejects(f.worker.execute({id: `invalid${index}`, kind: 'capture',
      sheets: [{...sheet, capture_end_column: end}]}), /Negotiated capture boundary/);
  }
});

test('evidence is recaptured only on demand and remains bound to its capture id', async () => {
  const f = fixture();
  const first = await f.worker.execute({id: 'first', kind: 'capture', sheets: [f.sheet]});
  await f.worker.execute({id: 'second', kind: 'capture', sheets: [{...f.sheet, tile_rows: [[3, 6]]}]});
  const image = await f.worker.execute({id: 'evidence', kind: 'images', capture_id: 'first', tiles: [first.tiles[3]]});
  assert.deepEqual(f.captures, ['0:3', '3:6', '6:9', '9:10', '3:6', '9:10']);
  assert.equal(f.uploads.length, 1);
  assert.equal(image.images[0].range, '9:10');
  assert.equal((await encoded(Buffer.from(f.uploads[0].bytes).toString('base64'))).sha256, first.tiles[3].sha256);
  await f.worker.execute({id: 'release', kind: 'release', capture_id: 'first'});
  await assert.rejects(f.worker.execute({id: 'old', kind: 'images', capture_id: 'first', tiles: [first.tiles[0]]}), /Unknown capture/);
});

test('changed workbook images fail instead of attributing a new image to the old hash', async () => {
  const f = fixture();
  const first = await f.worker.execute({id: 'first', kind: 'capture', sheets: [f.sheet]});
  f.change();
  await assert.rejects(f.worker.execute({id: 'evidence', kind: 'images', capture_id: 'first', tiles: [first.tiles[0]]}),
    error => error.code === 'CAPTURE_CHANGED');
  assert.equal(f.uploads.length, 0);
});

test('old sheet geometry is evicted and rebuilt only when evidence is requested', async () => {
  const f = fixture();
  const result = await f.worker.execute({id: 'first', kind: 'capture',
    sheets: [1, 2, 3].map(index => ({...f.sheet, index, name: `Sheet${index}`}))});
  assert.equal(f.geometriesCreated, 3);
  await f.worker.execute({id: 'evidence', kind: 'images', capture_id: 'first', tiles: [result.tiles[0]]});
  assert.equal(f.geometriesCreated, 4);
  assert.equal(f.uploads.length, 1);
});

test('invalid or overlapping requested spans are rejected', () => {
  fixture();
  const geometry = new CaptureGeometry.SheetGeometry();
  for (const spans of [[], [[3, 6], [5, 8]], [[-1, 2]], [[0, 11]], [[1, 1]]]) {
    assert.throws(() => selectedTiles(geometry, 1, spans), /Invalid requested/);
  }
});

test('image-first planning captures no images and uploads one requested tile without calling crypto', async () => {
  const f = fixture();
  const digest = globalThis.crypto.subtle.digest;
  globalThis.crypto.subtle.digest = () => {throw new Error('Client hashing is forbidden');};
  try {
    const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
    assert.equal(plan.tiles.length, 4);
    assert.equal(plan.checksum_tiles, 0);
    assert.deepEqual(f.captures, []);
    const image = await f.worker.execute({id: 'tile',kind: 'capture_images',capture_id: 'plan',tiles:[plan.tiles[2]]});
    assert.deepEqual(f.captures,['6:9']);
    assert.equal(f.uploads.length,1);
    assert.equal(f.uploads[0].metadata.sha256,undefined);
    assert.equal(image.images[0].sha256,'a'.repeat(64));
    assert.equal(image.uploaded_images,1);
  } finally {globalThis.crypto.subtle.digest = digest;}
});

test('image-first whitespace controls render only negotiated mismatch rows', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id:'control',kind:'plan',sheets:[{...f.sheet,tile_rows:[[3,6],[9,10]]}]});
  assert.deepEqual(f.captures,[]);
  await f.worker.execute({id:'control-first',kind:'capture_images',capture_id:'control',tiles:[plan.tiles[0]]});
  await f.worker.execute({id:'control-second',kind:'capture_images',capture_id:'control',tiles:[plan.tiles[1]]});
  assert.deepEqual(f.captures,['3:6','9:10']);
});

test('image-first rejects multiple tiles or geometry that differs from its plan', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id:'plan',kind:'plan',sheets:[f.sheet]});
  await assert.rejects(f.worker.execute({id:'many',kind:'capture_images',capture_id:'plan',tiles:plan.tiles}),/exactly one/);
  await assert.rejects(f.worker.execute({id:'wrong',kind:'capture_images',capture_id:'plan',tiles:[{...plan.tiles[0],width:99}]}),/differs from/);
  assert.deepEqual(f.captures,[]);
});
