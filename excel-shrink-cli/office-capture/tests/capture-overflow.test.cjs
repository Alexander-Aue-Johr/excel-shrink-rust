const test = require('node:test');
const assert = require('node:assert/strict');
const {check, crop, decode, release, sameImage} = require('../web/capture-overflow.js');

function overflowFixture(options = {}) {
  let value = '';
  const writes = [];
  const canvases = [];
  const images = [], contexts = [], draws = [];
  let pixelReads = 0;
  const candidateRows = options.candidateRows ?? [0];
  const rowCount = Math.max(...candidateRows) + 1;
  const height = rowCount * 4;
  const blank = new Uint8ClampedArray(12 * height * 4).fill(255);
  const text = blank.slice();
  for (const row of candidateRows) {
    text.set([0, 0, 0, 255], ((row * 4 + 1) * 12 + (options.textX ?? 4)) * 4);
  }
  globalThis.Image = class {
    constructor() { images.push(this); }
    naturalWidth = 12;
    naturalHeight = height;
    get src() { return this.source; }
    set src(source) {
      this.source = source;
      if (!source) return;
      this.pixels = source.endsWith(',blocked') ? blank : text;
      Promise.resolve().then(() => options.decodeFails ? this.onerror() : this.onload());
    }
  };
  globalThis.document = {createElement() {
    let pixels;
    let contextLost = options.contextLost ?? false;
    const canvas = {getContext(type, settings) {
      contexts.push({type, settings});
      if (options.noContext) return null;
      return {
        drawImage(image, ...rect) {pixels = image.pixels; draws.push({image, rect});},
        getImageData() {pixelReads++; return {data: pixels};},
        isContextLost() {return contextLost;}
      };
    }, toBlob(callback) {
      assert.ok(images.every(image => image.src === ''), 'release decoded images before encoding');
      queueMicrotask(() => {
        if (options.loseContextDuringEncoding) contextLost = true;
        callback(options.nullBlob ? null : {type: 'image/png'});
      });
    }};
    canvases.push(canvas);
    return canvas;
  }};
  globalThis.FileReader = class {
    readAsDataURL() {this.result = 'data:image/png;base64,encoded'; queueMicrotask(() => this.onload());}
  };
  globalThis.CaptureGeometry = {address() {return 'A1:B1';}, pngSize() {return {width: 12, height};}};

  const range = {
    load() {},
    get values() {return [[value]];},
    set values(cells) {
      writes.push(cells[0][0]);
      if ((cells[0][0] === ' ' && options.skipWrite) ||
          (cells[0][0] === '' && options.skipRestore)) return;
      value = cells[0][0];
    },
    get formulas() {return [[value === ' ' && options.formula ? '=" "' : value]];},
    get valueTypes() {return [[value === '' ? 'Empty' : 'String']];},
    hasSpill: false,
    format: {load() {}, textOrientation: 0},
    getMergedAreasOrNullObject() {return {load() {}, isNullObject: true};},
    getTables() {return {load() {}, items: []};},
    getPivotTables() {return {load() {}, items: []};}
  };
  const worksheet = {
    protection: {load() {}, protected: false},
    getRangeByIndexes(row, column, rows, columns) {
      const source = options.sourceContexts?.[row];
      if (column === 0 && columns === 1 && source) return {
        format: {load() {}, textOrientation: source.orientation === undefined ? 0 : source.orientation},
        getMergedAreasOrNullObject() {return {load() {}, isNullObject: !source.mergeRows,
          areas: {items: [{rowCount: source.mergeRows}]}};}
      };
      return range;
    },
    getRange() {return {getImage() {return {value: value === ' ' ? 'blocked' : 'original'};}};}
  };
  const office = fn => fn({sync: async () => {}, workbook: {worksheets: {getItem: () => worksheet}}});
  const geometry = {x: [0, 2, 6, 12], y: Array.from({length: rowCount + 1}, (_, i) => i * (options.pointRowHeight ?? 4)),
    pixelY: Array.from({length: rowCount + 1}, (_, i) => i * 4),
    columns: [2, 4, 6], scaleX: 1, scaleY: 1};
  if (options.hiddenLastColumn) { geometry.columns.push(0); geometry.x.push(12); }
  const tile = {raw: {r0: 0, r1: rowCount, c0: 0, c1: geometry.columns.length},
    core: {r0: 0, r1: rowCount}, cropY: 0, cropHeight: height};
  return {run: () => check(office, 'Sheet1', geometry, tile, 0, 'original'), writes,
    canvases, images, contexts, draws, pixelReads: () => pixelReads};
}

test('overflow blockers accept literal values returned through formulas and restore the cells', async () => {
  const fixture = overflowFixture();
  assert.deepEqual(await fixture.run(), {retainProbe: true, rightEdge: false});
  assert.deepEqual(fixture.writes, [' ', '']);
});

test('overflow candidates and change bands use rendered rows despite fractional point heights', async () => {
  const fixture = overflowFixture({candidateRows: [10], pointRowHeight: 4.9});
  await fixture.run();
  assert.deepEqual(fixture.writes, [' ', '']);
  const unsafe = overflowFixture({candidateRows: [10], pointRowHeight: 4.9,
    sourceContexts: {10: {mergeRows: 2}}});
  await assert.rejects(unsafe.run(), /Sheet1!11: merged cells spanning 2 rows/);
  assert.deepEqual(unsafe.writes, []);
});

test('overflow source diagnostics identify the rejected row within a batch and its multirow merge', async () => {
  const fixture = overflowFixture({candidateRows: [0, 1], sourceContexts: {1: {mergeRows: 3}}});
  await assert.rejects(fixture.run(), /Sheet1!2: merged cells spanning 3 rows/);
  assert.deepEqual(fixture.writes, []);
});

test('overflow source diagnostics distinguish rotated and mixed text without changing cells', async () => {
  for (const [orientation, reason] of [[45, 'text orientation 45 degrees'], [null, 'mixed or unavailable text orientation']]) {
    const fixture = overflowFixture({sourceContexts: {0: {orientation}}});
    await assert.rejects(fixture.run(), new RegExp(`Sheet1!1: ${reason}`));
    assert.deepEqual(fixture.writes, []);
  }
});

test('overflow check rejects a missing blocker write', async () => {
  const fixture = overflowFixture({skipWrite: true});
  await assert.rejects(fixture.run(), /did not write cells/);
  assert.deepEqual(fixture.writes, [' ', '']);
});

test('overflow in the final visible column is unresolved even with blank edge pixels', async () => {
  for (const hiddenLastColumn of [false, true]) {
    const fixture = overflowFixture({textX: 7, hiddenLastColumn});
    assert.deepEqual(await fixture.run(), {retainProbe: true, rightEdge: true});
    assert.deepEqual(fixture.writes, [' ', '']);
  }
});

test('overflow blockers must be literal cells rather than formulas returning spaces', async () => {
  await assert.rejects(overflowFixture({formula: true}).run(), /did not write cells/);
});

test('overflow check rejects cells that were not restored', async () => {
  await assert.rejects(overflowFixture({skipRestore: true}).run(), /did not restore cells/);
});

test('image comparison yields to the host and still detects a late changed pixel', async () => {
  const left = {width: 1024, height: 1024, pixels: new Uint8ClampedArray(4 * 1024 * 1024)};
  const right = {...left, pixels: left.pixels.slice()};
  let hostTurn = false;
  setImmediate(() => { hostTurn = true; });
  assert.equal(await sameImage(left, right), true);
  assert.equal(hostTurn, true);
  right.pixels[right.pixels.length - 1] = 1;
  assert.equal(await sameImage(left, right), false);
});

test('cropping uses one software canvas and releases the source before encoding', async () => {
  const fixture = overflowFixture();
  assert.equal(await crop('original', 2, 1, 6, 2), 'encoded');
  assert.equal(fixture.pixelReads(), 0);
  assert.equal(fixture.canvases.length, 1);
  assert.deepEqual(fixture.contexts, [{type: '2d', settings: {willReadFrequently: true}}]);
  assert.deepEqual(fixture.draws[0].rect, [2, 1, 6, 2, 0, 0, 6, 2]);
  assert.equal(fixture.draws[0].image, fixture.images[0]);
  assert.ok(fixture.canvases.every(canvas => canvas.width === 0 && canvas.height === 0));
});

test('a complete tile preserves the PNG without decoding an image or allocating a canvas', async () => {
  const fixture = overflowFixture();
  assert.equal(await crop('original', 0, 0, 12, 4), 'original');
  assert.equal(fixture.canvases.length, 0);
  assert.equal(fixture.images.length, 0);
});

test('encoding and context failures request smaller tiles instead of accepting missing pixels', async () => {
  for (const options of [{nullBlob: true}, {noContext: true}, {contextLost: true}, {loseContextDuringEncoding: true}]) {
    const fixture = overflowFixture(options);
    await assert.rejects(crop('original', 0, 1, 12, 2), error =>
      error.code === 'IMAGE_BUDGET' && /12 x 2; source 12 x 4/.test(error.message));
    assert.ok(fixture.canvases.every(canvas => canvas.width === 0 && canvas.height === 0));
    assert.equal(fixture.images[0].src, '');
  }
});

test('invalid crops fail before decoding an image', async () => {
  for (const rect of [[-1, 0, 12, 4], [0, 0, 13, 4], [0, 0, 12, 5], [0, .5, 12, 3], [0, 0, 0, 4]]) {
    const fixture = overflowFixture();
    await assert.rejects(crop('original', ...rect), error =>
      error.code !== 'IMAGE_BUDGET' && error.message === 'Invalid image crop');
    assert.equal(fixture.canvases.length, 0);
    assert.equal(fixture.images.length, 0);
  }
});

test('undecodable source images remain hard failures and detach their loading handlers', async () => {
  const fixture = overflowFixture({decodeFails: true});
  await assert.rejects(crop('broken', 0, 0, 12, 3), error =>
    error.code !== 'IMAGE_BUDGET' && error.message === 'Cannot decode Excel PNG');
  assert.equal(fixture.canvases.length, 0);
  assert.equal(fixture.images[0].src, '');
  assert.equal(fixture.images[0].onload, null);
  assert.equal(fixture.images[0].onerror, null);
});

test('pixel decoding releases the redundant canvas while retaining the pixel buffer', async () => {
  overflowFixture();
  const image = await decode('original');
  assert.equal(image.width, 12);
  assert.equal(image.height, 4);
  assert.equal(image.pixels.length, 12 * 4 * 4);
  assert.equal(image.canvas.width, 0);
  assert.equal(image.canvas.height, 0);
  release(image);
});
