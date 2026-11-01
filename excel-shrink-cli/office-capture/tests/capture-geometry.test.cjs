'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const {SheetGeometry, probeEnd, pngSize} = require('../web/capture-geometry.js');

test('overflow width uses measured pixels and skips hidden columns', () => {
  const measured = columns => {
    const x = [0];
    for (const width of columns) x.push(x.at(-1) + width);
    return {columns, x, scaleX: 4 / 3};
  };
  assert.equal(probeEnd(measured(new Array(65).fill(60)), 0), 24);
  assert.equal(probeEnd(measured([60, 0, 0, ...new Array(64).fill(120)]), 0), 14);
  assert.equal(probeEnd(measured(new Array(80).fill(1)), 0), 64);
  assert.throws(() => probeEnd(measured([60, 0, 0]), 0), /No visible column/);
});

test('drawing bounds allow arithmetic roundoff but reject a real overhang', () => {
  const {exceedsExtent} = require('../web/capture-geometry.js');
  assert.equal(exceedsExtent(789.750078740158 + 9.74992125984252, 799.5), false);
  assert.equal(exceedsExtent(799.500001, 799.5), true);
  assert.equal(exceedsExtent(800, 799.5), true);
  assert.throws(() => exceedsExtent(Infinity, 799.5), /Invalid drawing boundary/);
  assert.throws(() => exceedsExtent(799.5, NaN), /Invalid drawing boundary/);
});

function png(width, height) {
  const bytes = Buffer.alloc(57);
  Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]).copy(bytes);
  bytes.write('IHDR', 12);
  bytes.writeUInt32BE(width, 16);
  bytes.writeUInt32BE(height, 20);
  bytes.write('IEND', 49);
  return bytes.toString('base64');
}

test('large PNG dimensions are accepted only within the explicit capture limit', () => {
  assert.deepEqual(pngSize(png(8000, 6000)), {width: 8000, height: 6000});
  assert.deepEqual(pngSize(png(8000, 8000)), {width: 8000, height: 8000});
  assert.throws(() => pngSize(png(8001, 8000)), /capture budget/);
  const truncated = Buffer.from(png(8000, 6000), 'base64').subarray(0, -12).toString('base64');
  assert.throws(() => pngSize(truncated), /invalid or truncated PNG/);
});

// Office exposes point heights finer than a PNG pixel. Images round each row
// down independently; aggregate point heights are not image dimensions.
function officeModel({rows = [], columns = [], defaultHeight = 14, defaultWidth = 60,
    scale = 4 / 3, imageWidthOffset = 0, imageHeightOffset = 0, imageHeight} = {}) {
  const images = [];
  const rangeRequests = [];
  const row = index => rows[index] ?? defaultHeight;
  const column = index => columns[index] ?? defaultWidth;
  const pointHeight = index => Math.round(row(index) * 16 / 3) * 3 / 16;
  const sum = (value, start, count) => {
    let result = 0;
    for (let index = start; index < start + count; index++) result += value(index);
    return result;
  };
  const sheet = {
    getRangeByIndexes(r, c, nr, nc) {
      rangeRequests.push({r, c, nr, nc});
      return {
        load() {},
        width: sum(column, c, nc),
        height: nr === 1 ? pointHeight(r) : sum(row, r, nr),
        rowHidden: row(r) === 0,
        columnHidden: column(c) === 0,
        getImage() {
          images.push({r, c, nr, nc});
          return {value: png(Math.round(sum(column, c, nc) * scale) + 1 + imageWidthOffset,
            (imageHeight ? imageHeight(r, nr) : sum(index => Math.floor(pointHeight(index) * scale), r, nr) + 1) + imageHeightOffset)};
        }
      };
    }
  };
  return {images, rangeRequests,
    office: fn => fn({sync: async () => {}, workbook: {worksheets: {getItem: () => sheet}}})};
}

async function geometry(model, bottom = 99, right = 15) {
  const result = new SheetGeometry(model.office, 'Sheet', bottom, right);
  await result.read(right);
  await result.calibrate();
  return result;
}

test('measured height runs preserve full geometry without per-row Office reads', async () => {
  const options = {rows: [15, 15, 0, 18, 18]};
  const runs = [
    {start_row: 1, end_row: 2, height: 15},
    {start_row: 3, end_row: 3, height: 0},
    {start_row: 4, end_row: 5, height: 18},
    {start_row: 6, end_row: 10000, height: 14.0625}
  ];
  const fullModel = officeModel(options);
  const reusedModel = officeModel(options);
  const full = new SheetGeometry(fullModel.office, 'Sheet', 9999, 15);
  const reused = new SheetGeometry(reusedModel.office, 'Sheet', 9999, 15);
  await full.read(15);
  await reused.read(15, runs);
  await full.calibrate();
  await reused.calibrate();

  assert.deepEqual(reused.rows, full.rows);
  assert.deepEqual(reused.pixelY, full.pixelY);
  assert.deepEqual(reused.tiles(15, null), full.tiles(15, null));
  assert.ok(fullModel.rangeRequests.length > 10000);
  assert.ok(reusedModel.rangeRequests.length < 100);
  await assert.rejects(() => reused.read(15, runs.slice(1)), /Invalid measured row-height coverage/);
});

test('million-row planning precedes capture limits and sparse bands omit the middle', () => {
  // Calibrated geometry matching a sparse sheet with a last-row total. No
  // Office calls are needed to exercise the planner after those measurements.
  const rows = 1048574;
  const result = new SheetGeometry(null, 'Trial 3', rows - 1, 13, 4);
  result.rows = new Float64Array(rows).fill(12.75);
  result.pixelY = Float64Array.from({length: rows + 1}, (_, index) => index * 17);
  result.visibleRows = Uint32Array.from({length: rows + 1}, (_, index) => index);
  result.columns = new Float64Array(14).fill(100);
  result.x = Float64Array.from({length: 15}, (_, index) => index * 100);
  result.scaleX = result.scaleY = 4 / 3;
  result.fixedWidth = null;

  assert.throws(() => result.tiles(13, null), error => error.code === 'IMAGE_BUDGET');
  const plan = result.tiles(13, null, undefined, {planOnly: true});
  assert.ok(plan.length > 4096 && plan.length <= 65536);
  assert.equal(plan[0].core.r0, 0);
  assert.equal(plan.at(-1).core.r1, rows);
  const shared = plan.map(tile => [tile.core.r0, tile.core.r1]);
  assert.equal(result.tiles(13, null, shared, {planOnly: true}).length, plan.length);

  const wanted = [...shared.slice(0, 2), ...shared.slice(-2)];
  const {selectedTiles} = require('../web/resident.js');
  const captures = selectedTiles(result, 13, wanted);
  assert.deepEqual(captures.map(tile => [tile.core.r0, tile.core.r1]), wanted);
  assert.ok(captures.every(tile => tile.width * tile.expectedHeight <= result.maxPixels));
  assert.throws(() => selectedTiles(result, 13, shared), error => error.code === 'IMAGE_BUDGET');
  assert.throws(() => selectedTiles(result, 13, [[0, rows]]), /exceeds the image budget/);
  assert.throws(() => result.tiles(13, null, Array(65537).fill([0, 1]), {planOnly: true}),
    error => error.code === 'IMAGE_BUDGET' && /metadata budget/.test(error.message));
});

test('configurable 1 to 64 megapixel budgets refine real full-width guarded captures', async () => {
  const model = officeModel();
  const small = new SheetGeometry(model.office, 'Sheet', 999, 15, 1);
  await small.read(15); await small.calibrate();
  const large = await geometry(officeModel(), 999, 15);
  const tiles = small.tiles(15, null);
  assert.ok(tiles.length > large.tiles(15, null).length);
  assert.equal(tiles[0].core.r0,0);
  assert.equal(tiles.at(-1).core.r1,1000);
  assert.ok(tiles.every(tile => tile.raw.c0 === 0 && tile.raw.c1 === 16 && tile.width*tile.expectedHeight <= 1000000));
  for (const value of [0,-1,65,1.5,NaN]) {
    assert.throws(() => new SheetGeometry(model.office,'Sheet',99,15,value), /megapixel budget/);
  }
});

test('an opted-in larger tile preserves the full measured width and height', async () => {
  const model = officeModel({rows: new Array(180).fill(16), columns: new Array(100).fill(75)});
  const geometry = new SheetGeometry(model.office, 'Sheet', 179, 99, 48);
  await geometry.read(99);
  await geometry.calibrate();
  const tiles = geometry.tiles(99, null);
  assert.equal(tiles.length, 1);
  assert.ok(tiles[0].width * tiles[0].expectedHeight > 32000000);
  assert.ok(tiles[0].width * tiles[0].expectedHeight <= 48000000);
  assert.equal(tiles[0].width, geometry.imageWidth(99));
  assert.equal(tiles[0].expectedHeight, geometry.imageHeight(0, 180));
});

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

test('capture and calibration quantize individual row heights before summing', async () => {
  const result = await geometry(officeModel(), 9999);
  assert.equal(result.rows[0], 14.0625);
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

test('narrower paired columns retain the conservative row cuts and image budget', async () => {
  const result = await geometry(officeModel(), 9999, 65);
  const wide = result.tiles(65, null);
  const rows = wide.map(tile => [tile.core.r0, tile.core.r1]);
  const narrow = result.tiles(probeEnd(result, 1), null, rows);
  assert.deepEqual(narrow.map(tile => [tile.core.r0, tile.core.r1]), rows);
  for (let index = 0; index < narrow.length; index++) {
    assert.ok(narrow[index].width < wide[index].width);
    assert.ok(narrow[index].width * narrow[index].expectedHeight <= result.maxPixels);
  }
});

test('hidden rows and columns contribute no rendered extent', async () => {
  const result = await geometry(officeModel({rows: [0, 14, 0, 18], columns: [0, 60, 0, 75]}));
  assert.deepEqual(result.rows.slice(0, 4), [0, 14.0625, 0, 18]);
  assert.equal(result.imageHeight(0, 4), 43);
  assert.equal(result.imageWidth(3), 181);
});

test('a single visible column retains fixed full-width capture', async () => {
  const result = await geometry(officeModel({columns: [120], defaultWidth: 0}), 99, 0);
  assert.equal(result.scaleX, null);
  assert.equal(result.imageWidth(0), 161);
  assert.throws(() => result.imageWidth(1), /Fixed-width capture cannot crop columns/);
});

test('calibration crosses hidden columns to find distinct safe widths', async () => {
  const model = officeModel({columns: [60, 60, ...new Array(96).fill(0)], defaultWidth: 60});
  const result = await geometry(model, 99, 200);
  assert.equal(result.scaleX, 4 / 3);
  assert.equal(result.fixedWidth, null);
  assert.ok(model.images.some(control => control.nc === 99));
  assert.equal(result.imageWidth(98), 241);
  const tiles = result.tiles(200, null);
  assert.ok(tiles.every(tile => tile.width * tile.expectedHeight <= result.maxPixels));
});

test('calibration crosses long hidden row runs to find distinct visible heights', async () => {
  const rows = [15, ...new Array(4094).fill(0), 15, ...new Array(4094).fill(0), 15];
  const model = officeModel({rows});
  const result = await geometry(model, rows.length - 1, 3);
  assert.equal(result.scaleY, 4 / 3);
  assert.equal(result.imageHeight(0, rows.length), 61);
  assert.ok(model.images.some(control => control.nr > 32));
  assert.equal(result.tiles(3, null).at(-1).core.r1, rows.length);
});

test('wide default columns use bounded calibration before choosing the overflow margin', async () => {
  const model = officeModel({defaultWidth: 800});
  const result = await geometry(model, 12, 64);
  assert.equal(result.scaleX, 4 / 3);
  assert.equal(result.fixedWidth, null);
  assert.ok(model.images.some(control => control.nc >= 3));
  assert.equal(probeEnd(result, 0), 2);
  assert.equal(result.imageWidth(2), 3201);
  assert.ok(result.tiles(2, null).every(tile => tile.width * tile.expectedHeight <= result.maxPixels));
  assert.throws(() => result.tiles(64, null), /exceeds the image edge budget/);
});

test('a tall wide singleton calibrates with safe default cells outside its logical bounds', async () => {
  const model = officeModel({rows: [169], columns: [800]});
  const result = await geometry(model, 0, 0);
  assert.equal(result.scaleX, 4 / 3);
  assert.equal(result.scaleY, 4 / 3);
  assert.ok(model.images.some(control => control.r > 0));
  assert.ok(model.images.some(control => control.c > 0));
  assert.equal(result.tiles(0, null).length, 1);
  assert.deepEqual(result.tiles(0, null)[0].core, {r0: 0, r1: 1, c0: 0, c1: 1});
});

test('an indivisible tall row and its halo may exceed the tile target within the hard pixel limit', async () => {
  // Paleosol's first rows are 23/169/127 points. The halo needs the next
  // whole row even though only two pixels of that row provide context.
  const model = officeModel({rows: [23, 169, 127, ...new Array(40).fill(19)], defaultWidth: 120});
  const result = new SheetGeometry(model.office, 'Sheet', 42, 161, 4);
  await result.read(161); await result.calibrate();
  const tiles = result.tiles(161, null);
  assert.equal(result.maxPixels, 4000000);
  assert.equal(tiles[0].core.r1, 1);
  assert.equal(tiles[0].budgetFallback, 'single-visible-row');
  assert.equal(tiles[0].pixelBudget, 32000000);
  assert.ok(tiles[0].width * tiles[0].expectedHeight > result.maxPixels);
  assert.ok(tiles.every(tile => tile.width * tile.expectedHeight <= tile.pixelBudget));
  assert.ok(tiles.filter(tile => !tile.budgetFallback).every(tile => tile.pixelBudget === result.maxPixels));
  const sharedRows = tiles.map(tile => [tile.core.r0, tile.core.r1]);
  assert.deepEqual(result.tiles(161, null, sharedRows), tiles);
  assert.equal(tiles.at(-1).core.r1, result.rows.length);
});

test('one-row fallback never admits a larger shared multi-row request', async () => {
  const model = officeModel({rows: [252, 92, 243, ...new Array(32).fill(16)], defaultWidth: 60});
  const result = new SheetGeometry(model.office, 'Sheet', 34, 179, 4);
  await result.read(179); await result.calibrate();
  const tiles = result.tiles(179, null);
  assert.equal(tiles[0].budgetFallback, 'single-visible-row');
  assert.throws(() => result.tiles(179, null, [[0, 2], ...tiles.slice(2).map(tile => [tile.core.r0, tile.core.r1])]), /exceeds the image budget/);
});

test('even a single row remains rejected when its guarded image exceeds 32 megapixels', async () => {
  const model = officeModel({rows: [409, 409, 409, ...new Array(32).fill(15)],
    columns: [10, 10, 10, ...new Array(220).fill(100)]});
  const result = new SheetGeometry(model.office, 'Sheet', 34, 222, 4);
  await result.read(222); await result.calibrate();
  assert.ok(result.imageWidth(222) < 30000);
  const halo = result.halo(0, 1);
  assert.ok(result.imageWidth(222) * result.imageHeight(halo.first, halo.last) > 32000000);
  assert.throws(() => result.tiles(222, null), /Row 1 exceeds the image budget/);
});

test('calibration still rejects images that disagree with measured geometry', async () => {
  await assert.rejects(geometry(officeModel({imageWidthOffset: 3})), /Inconsistent width image calibration/);
});

test('inconsistent calibration reports the measured points and image pixels', async () => {
  await assert.rejects(geometry(officeModel({imageHeightOffset: 3})), error => {
    assert.match(error.message, /Inconsistent height image calibration \(points, pixels\):/);
    const samples = JSON.parse(error.message.split(': ')[1]);
    assert.ok(samples.some(([points, pixels]) => points === 14.0625 && pixels === 22));
    assert.ok(samples.some(([points, pixels]) => points === 28.125 && pixels === 40));
    return true;
  });
});

test('fractional row calibration matches native Data.gov PNG measurements exactly', async () => {
  // Native paired captures from 2026-10-22. Stored row heights, rounded to the
  // Office API's 3/16-point grid, reproduce all seven measured point sums.
  // PNG control heights below are fixed observations, not model calculations.
  const fixtures = [{
    rows: [27.5625, 20.25, 20.25, ...new Array(5).fill(20.0625), 44.0625,
      ...new Array(4).fill(20.0625), ...new Array(19).fill(19.875)],
    bottom: 12, single: 27,
    points: [27.5625, 47.8125, 68.0625, 88.125, 168.375, 352.3125, 670.3125],
    pixels: [37, 64, 91, 117, 221, 461, 877]
  }, {
    rows: [13.6875, 38.25, 12.75, 12.75, 12.75, 25.5, 38.25,
      13.6875, 13.6875, 13.6875, 24.5625, ...new Array(10).fill(13.6875), ...new Array(11).fill(12)],
    bottom: 20, single: 18,
    points: [13.6875, 51.9375, 64.6875, 77.4375, 167.625, 288, 488.4375],
    pixels: [19, 70, 87, 104, 224, 382, 648]
  }];
  const counts = [1, 2, 3, 4, 8, 16, 32];
  for (const fixture of fixtures) {
    assert.deepEqual(counts.map(count => fixture.rows.slice(0, count).reduce((sum, height) => sum + height, 0)), fixture.points);
    const result = await geometry(officeModel({rows: fixture.rows, imageHeight(row, count) {
      if (row === 0) return fixture.pixels[counts.indexOf(count)];
      assert.equal(count, 1);
      return fixture.single;
    }}), fixture.bottom);
    assert.equal(result.scaleY, 4 / 3);
    for (let index = 0; index < counts.length; index++) {
      if (counts[index] <= fixture.bottom + 1) assert.equal(result.imageHeight(0, counts[index]), fixture.pixels[index]);
    }
  }
});

test('row calibration derives rational image scales without assuming 96 DPI', async () => {
  for (const scale of [1, 5 / 4, 4 / 3, 2]) {
    const rows = Array.from({length: 64}, (_, row) => [15, 23.625, 12.9375, 18.375][row % 4]);
    const result = await geometry(officeModel({rows, scale}), rows.length - 1);
    assert.equal(result.scaleY, scale);
    assert.equal(result.imageHeight(0, rows.length), rows.reduce((sum, height) => sum + Math.floor(height * scale), 1));
  }
});

test('fractional rows and hidden runs keep exact pixel crops, halos and shared cuts', async () => {
  const rows = Array.from({length: 12000}, (_, row) => [13.6875, 0, 20.0625, 38.25][row % 4]);
  const result = await geometry(officeModel({rows}), rows.length - 1, 65);
  const tiles = result.tiles(65, null);
  assert.ok(tiles.length > 1);
  for (const tile of tiles) {
    const sum = (start, end) => rows.slice(start, end).reduce((total, height) => total + Math.floor(height * 4 / 3), 0);
    assert.equal(tile.cropY, sum(tile.raw.r0, tile.core.r0));
    assert.equal(tile.cropHeight, sum(tile.core.r0, tile.core.r1) + 1);
    assert.equal(tile.expectedHeight, sum(tile.raw.r0, tile.raw.r1) + 1);
    assert.ok(tile.cropY + tile.cropHeight <= tile.expectedHeight);
    if (tile.core.r0 > 0) assert.ok(tile.cropY >= 2);
    if (tile.core.r1 < rows.length) assert.ok(tile.expectedHeight - tile.cropY - tile.cropHeight >= 2);
  }
  const shared = tiles.map(tile => [tile.core.r0, tile.core.r1]);
  assert.deepEqual(result.tiles(65, null, shared), tiles);
  assert.notEqual(result.pixelY.at(-1), Math.round(result.y.at(-1) * result.scaleY));
});

test('wide leading columns calibrate without a full-width image', async () => {
  const model = officeModel({columns: [0, 39.75, 504.75, 312, 222.75], defaultWidth: 60});
  const result = await geometry(model, 864, 70);
  assert.equal(result.scaleX, 4 / 3);
  assert.equal(result.imageWidth(70), 6720);
  assert.ok(model.images.some(control => control.nc === 3));
  assert.ok(model.images.every(control => control.nc <= 32));
});

test('drawing bounds include the rendered border pixel without allowing clipping', async () => {
  const result = await geometry(officeModel({
    columns: [60, 60, 63.75, 65.25, 60, 62.25, 60, 60], defaultHeight: 9.75
  }), 90, 7);
  assert.equal(result.x[8], 491.25);
  assert.equal(result.imageWidth(7), 656);
  assert.equal(result.containsExtent(306.9 + 184.651338582677, 77.25, 7), true);
  assert.equal(result.containsExtent(492.01, 77.25, 7), false);
  const bottom = result.imageHeight(0, result.rows.length) / result.scaleY;
  assert.equal(result.containsExtent(491.25, bottom, 7), true);
  assert.equal(result.containsExtent(491.25, bottom + 0.01, 7), false);
});

test('a fixed-width image retains the measured drawing boundary', async () => {
  const result = await geometry(officeModel({columns: [120], defaultWidth: 0}), 99, 0);
  assert.equal(result.containsExtent(120, 100, 0), true);
  assert.equal(result.containsExtent(120.01, 100, 0), false);
});

function deferredLayoutModel() {
  const sheet = {
    getRangeByIndexes(row, column, rowCount, columnCount) {
      let layoutResolved = false;
      return {
        load(properties) { layoutResolved = properties.split(',').includes('top'); },
        get height() {
          const containsPendingRow = row <= 64 && row + rowCount > 64;
          return rowCount * 12 + (containsPendingRow && !layoutResolved ? 1535.25 - 12 : 0);
        },
        width: columnCount * 60,
        rowCount, rowHidden: false, columnHidden: false,
        format: {load() {}, rowHeight: 12}
      };
    }
  };
  return fn => fn({sync: async () => {}, workbook: {worksheets: {getItem: () => sheet}}});
}

test('capture geometry resolves pending layout before its first height measurement', async () => {
  const result = new SheetGeometry(deferredLayoutModel(), 'Sheet', 99, 1);
  await result.read(1);
  assert.equal(result.rows[64], 12);
  assert.equal(result.y[100], 1200);
});

test('the row-height gate resolves pending layout without taking images', async () => {
  const {readHeightRuns} = require('../web/capture-geometry.js');
  const runs = await readHeightRuns(deferredLayoutModel(), 'Sheet', 99);
  assert.deepEqual(runs, [{start_row: 1, end_row: 100, height: 12}]);
});

test('height checks preserve absolute row coordinates when excluding fixed-height spans', async () => {
  const {readHeightRuns} = require('../web/capture-geometry.js');
  const runs = await readHeightRuns(deferredLayoutModel(), 'Sheet', 99, 50);
  assert.deepEqual(runs, [{start_row: 51, end_row: 100, height: 12}]);
  await assert.rejects(readHeightRuns(deferredLayoutModel(), 'Sheet', 99, 100), /Invalid first row/);
});

test('a long hidden tail stays covered by one visible image', async () => {
  const rows = [...new Array(94).fill(12.75), ...new Array(65530 - 94).fill(0)];
  const result = await geometry(officeModel({rows}), 65529, 1);
  const tiles = result.tiles(1, null);
  assert.equal(tiles.length, 1);
  assert.deepEqual(tiles[0].core, {r0: 0, r1: 65530, c0: 0, c1: 2});
  assert.equal(tiles[0].expectedHeight, 1599);
});

test('tiles cross hidden row runs with complete coverage and visible context', async () => {
  const rows = [...new Array(4000).fill(12), ...new Array(30000).fill(0), ...new Array(4000).fill(12)];
  const result = await geometry(officeModel({rows}), rows.length - 1, 1);
  const tiles = result.tiles(1, null);
  assert.ok(tiles.length > 1);
  let nextRow = 0;
  for (const tile of tiles) {
    assert.equal(tile.core.r0, nextRow);
    nextRow = tile.core.r1;
    assert.ok(tile.cropHeight > 1);
    assert.ok(tile.cropY + tile.cropHeight <= tile.expectedHeight);
    assert.ok(tile.width * tile.expectedHeight <= result.maxPixels);
    assert.ok(tile.expectedHeight <= 30000);
    assert.ok(result.visibleRows[tile.core.r1] - result.visibleRows[tile.core.r0] <= 8192);
  }
  assert.equal(nextRow, rows.length);
});

test('the tile row limit still bounds visible rows when pixels fit', async () => {
  const result = await geometry(officeModel({defaultHeight: 0.75}), 9999, 1);
  const tiles = result.tiles(1, null);
  assert.equal(tiles.length, 2);
  assert.equal(tiles[0].core.r1, 8192);
  assert.equal(tiles[1].core.r0, 8192);
  assert.equal(tiles[1].core.r1, 10000);
});

test('calibration includes visible cells before the smallest row and column', async () => {
  const result = await geometry(officeModel({rows: [24, 24, 12], defaultHeight: 0,
    columns: [120, 120, 60], defaultWidth: 0}), 2, 2);
  assert.equal(result.scaleX, 4 / 3);
  assert.equal(result.scaleY, 4 / 3);
  assert.equal(result.imageWidth(2), 401);
  assert.equal(result.imageHeight(0, 3), 81);
});

test('shared row boundaries keep both image budgets when empty margin widths differ', async () => {
  const left = await geometry(officeModel({defaultWidth: 60}), 9999);
  const right = await geometry(officeModel({columns: [...new Array(15).fill(60), 54.75]}), 9999);
  const initial = [left, right].map(side => side.tiles(15, null));
  assert.notDeepEqual(initial[0].map(tile => tile.core), initial[1].map(tile => tile.core));
  const cuts = [...new Set(initial.flat().flatMap(tile => [tile.core.r0, tile.core.r1]))].sort((a, b) => a - b);
  const rows = cuts.slice(1).map((end, index) => [cuts[index], end]);
  const paired = [left, right].map(side => side.tiles(15, null, rows));
  assert.deepEqual(paired[0].map(tile => tile.core), paired[1].map(tile => tile.core));
  for (const [index, tiles] of paired.entries()) {
    assert.ok(tiles.every(tile => tile.width * tile.expectedHeight <= [left, right][index].maxPixels));
  }
});

test('shared tiles preserve selected-row coverage without capturing the gaps', async () => {
  const result = await geometry(officeModel(), 199);
  const tiles = result.tiles(15, [3, 100], [[1, 2], [2, 4], [98, 101]]);
  assert.deepEqual(tiles.map(tile => [tile.core.r0, tile.core.r1]), [[1, 2], [2, 4], [98, 101]]);
  assert.throws(() => result.tiles(15, [3, 100], [[1, 4], [98, 100]]), /do not cover/);
});

test('shared tile rows reject gaps, overlap and excessive image sizes', async () => {
  const result = await geometry(officeModel(), 9999);
  assert.throws(() => result.tiles(15, null, [[0, 9999]]), /do not cover/);
  assert.throws(() => result.tiles(15, null, [[0, 100], [99, 10000]]), /Invalid shared/);
  assert.throws(() => result.tiles(15, null, [[0, 10001]]), /Invalid shared/);
  assert.throws(() => result.tiles(15, null, [[0, 10000]]), /exceeds the image budget/);
});

function marginGeometry(widthAt, base = 0, scale = 1) {
  const geometry = new SheetGeometry(null, 'Sheet', 0, base);
  geometry.columns = Array.from({length: base + 1}, (_, column) => widthAt(column));
  geometry.x = [0];
  for (const width of geometry.columns) geometry.x.push(geometry.x.at(-1) + width);
  geometry.scaleX = scale;
  geometry.fixedWidth = null;
  geometry.readSizes = async (axis, start, count) => {
    assert.equal(axis, 'columns');
    return Array.from({length: count}, (_, offset) => widthAt(start + offset));
  };
  return geometry;
}

test('fixed margin reaches 1920 measured pixels without a 64-column cutoff', async () => {
  const geometry = marginGeometry(column => column === 0 ? 60 : 1);
  const end = await geometry.rightMarginEnd(0);
  assert.equal(end, 1920);
  assert.equal(geometry.imageWidth(end) - geometry.imageWidth(0), 1920);
});

test('fixed margin skips long hidden spans and rounds up to complete columns', async () => {
  const hidden = marginGeometry(column => column === 0 ? 60 : column <= 800 ? 0 : 80);
  assert.equal(await hidden.rightMarginEnd(0), 824);
  const scaled = marginGeometry(() => 100, 0, 4 / 3);
  const end = await scaled.rightMarginEnd(0);
  assert.ok(scaled.imageWidth(end) - scaled.imageWidth(0) >= 1920);
  assert.ok(scaled.imageWidth(end - 1) - scaled.imageWidth(0) < 1920);
});

test('fixed margin stops at XFD and rejects unmeasurable scaling', async () => {
  const geometry = marginGeometry(() => 40, 16381);
  assert.equal(await geometry.rightMarginEnd(16381), 16383);
  assert.equal(geometry.imageWidth(16383) - geometry.imageWidth(16381), 80);
  assert.equal(await geometry.rightMarginEnd(16383), 16383);
  geometry.fixedWidth = 800;
  await assert.rejects(geometry.rightMarginEnd(16381), /Cannot measure fixed right/);
});
