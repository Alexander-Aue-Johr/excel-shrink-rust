'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const {SheetGeometry} = require('../web/capture-geometry.js');

function png(width, height) {
  const bytes = Buffer.alloc(57);
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]).copy(bytes);
  bytes.write('IHDR', 12);
  bytes.writeUInt32BE(width, 16);
  bytes.writeUInt32BE(height, 20);
  bytes.write('IEND', 49);
  return bytes.toString('base64');
}

// Office can expose stored fractional heights on a multirow range while single
// cells and PNGs use rounded visible heights. These are model tests, not Excel.
function officeModel({rows = [], columns = [], defaultHeight = 14, defaultWidth = 60,
    scale = 4 / 3, imageWidthOffset = 0} = {}) {
  const images = [];
  const row = index => rows[index] ?? defaultHeight;
  const column = index => columns[index] ?? defaultWidth;
  const visibleHeight = index => Math.floor(row(index) * 4 / 3) * 3 / 4;
  const sum = (value, start, count) => {
    let result = 0;
    for (let index = start; index < start + count; index++) result += value(index);
    return result;
  };
  const sheet = {
    getRangeByIndexes(r, c, nr, nc) {
      return {
        load() {},
        width: sum(column, c, nc),
        height: nr === 1 ? visibleHeight(r) : sum(row, r, nr),
        rowHidden: row(r) === 0,
        columnHidden: column(c) === 0,
        getImage() {
          images.push({r, c, nr, nc});
          return {value: png(Math.round(sum(column, c, nc) * scale) + 1 + imageWidthOffset,
            Math.round(sum(visibleHeight, r, nr) * scale) + 1)};
        }
      };
    }
  };
  return {images, office: fn => fn({sync: async () => {}, workbook: {worksheets: {getItem: () => sheet}}})};
}

async function geometry(model, bottom = 99, right = 15) {
  const result = new SheetGeometry(model.office, 'Sheet', bottom, right);
  await result.read(right);
  await result.calibrate();
  return result;
}

test('tall title rows allow independent narrow calibration controls', async () => {
  const model = officeModel({rows: [129.75, 24, 68.25, 30, 63.75, 90.75, 116.25, 20.25],
    columns: [138], defaultWidth: 63.75});
  const result = await geometry(model, 7, 77);
  assert.equal(result.scaleX, 4 / 3);
  assert.equal(result.scaleY, 4 / 3);
  assert.equal(result.imageHeight(0, 8), 725);
  assert.ok(model.images.some(control => control.nr > 1 && control.nc === 1));
  assert.ok(model.images.some(control => control.nc > 1 && control.nr === 1));
});

test('capture and calibration sum individual visible row heights', async () => {
  const result = await geometry(officeModel(), 9999);
  assert.equal(result.rows[0], 13.5);
  assert.equal(result.scaleY, 4 / 3);
  assert.equal(result.imageHeight(0, 10000), 180001);
  const tiles = result.tiles(15, null);
  assert.equal(tiles[0].core.r0, 0);
  assert.equal(tiles.at(-1).core.r1, 10000);
  assert.ok(tiles.length > 1);
  for (const tile of tiles) {
    assert.equal(tile.expectedHeight, (tile.raw.r1 - tile.raw.r0) * 18 + 1);
    assert.equal(tile.cropHeight, (tile.core.r1 - tile.core.r0) * 18 + 1);
    assert.ok(tile.cropY + tile.cropHeight <= tile.expectedHeight);
    assert.ok(tile.width * tile.expectedHeight <= result.maxPixels);
  }
});

test('hidden rows and columns contribute no rendered extent', async () => {
  const result = await geometry(officeModel({rows: [0, 14, 0, 18], columns: [0, 60, 0, 75]}));
  assert.deepEqual(result.rows.slice(0, 4), [0, 13.5, 0, 18]);
  assert.equal(result.imageHeight(0, 4), 43);
  assert.equal(result.imageWidth(3), 181);
});

test('a single visible column retains fixed full-width capture', async () => {
  const result = await geometry(officeModel({columns: [120], defaultWidth: 0}), 99, 0);
  assert.equal(result.scaleX, null);
  assert.equal(result.imageWidth(0), 161);
  assert.throws(() => result.imageWidth(1), /Fixed-width capture cannot crop columns/);
});

test('calibration still rejects images that disagree with measured geometry', async () => {
  await assert.rejects(geometry(officeModel({imageWidthOffset: 3})), /Inconsistent width image calibration/);
});
