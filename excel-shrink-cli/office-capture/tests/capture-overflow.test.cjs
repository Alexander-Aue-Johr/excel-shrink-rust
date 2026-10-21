const test = require('node:test');
const assert = require('node:assert/strict');
const {check} = require('../web/capture-overflow.js');

function overflowFixture(options = {}) {
  let value = '';
  const writes = [];
  const blank = new Uint8ClampedArray(12 * 4 * 4).fill(255);
  const text = blank.slice();
  text.set([0, 0, 0, 255], (1 * 12 + (options.textX ?? 4)) * 4);
  globalThis.Image = class {
    naturalWidth = 12;
    naturalHeight = 4;
    set src(source) {
      this.pixels = source.endsWith(',blocked') ? blank : text;
      Promise.resolve().then(() => this.onload());
    }
  };
  globalThis.document = {createElement() {
    let pixels;
    return {getContext() {return {
      drawImage(image) {pixels = image.pixels;},
      getImageData() {return {data: pixels};}
    };}};
  }};
  globalThis.CaptureGeometry = {address() {return 'A1:B1';}};

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
    getRangeByIndexes() {return range;},
    getRange() {return {getImage() {return {value: value === ' ' ? 'blocked' : 'original'};}};}
  };
  const office = fn => fn({sync: async () => {}, workbook: {worksheets: {getItem: () => worksheet}}});
  const geometry = {x: [0, 2, 6, 12], y: [0, 4], columns: [2, 4, 6], scaleX: 1, scaleY: 1};
  if (options.hiddenLastColumn) { geometry.columns.push(0); geometry.x.push(12); }
  const tile = {raw: {r0: 0, r1: 1, c0: 0, c1: geometry.columns.length}, core: {r0: 0, r1: 1}, cropY: 0, cropHeight: 4};
  return {run: () => check(office, 'Sheet1', geometry, tile, 0, 'original'), writes};
}

test('overflow blockers accept literal values returned through formulas and restore the cells', async () => {
  const fixture = overflowFixture();
  assert.deepEqual(await fixture.run(), {retainProbe: true, rightEdge: false});
  assert.deepEqual(fixture.writes, [' ', '']);
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
