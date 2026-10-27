const test = require('node:test');
const assert = require('node:assert/strict');
const {createHash} = require('node:crypto');
const {createWorker, selectedTiles, encoded} = require('../web/resident.js');

function fixture(workbookName = 'fixture.xlsx', hooks = {}) {
  const captures = [], uploads = [], heightSheets = [], geometryReads = [], captureBatches = [];
  const events = [];
  let geometriesCreated = 0;
  let officeActive = 0;
  let peakOfficeActive = 0;
  let content = 'stable';
  const encode = (width, height) => Buffer.from(JSON.stringify({width, height, content})).toString('base64');
  globalThis.CaptureGeometry = {
    address: rect => `${rect.r0}:${rect.r1}`,
    pngSize: value => JSON.parse(Buffer.from(value, 'base64').toString()),
    probeEnd: () => 3,
    SheetGeometry: class {
      constructor() {
        geometriesCreated++;
        this.rows = Array(hooks.rowCount ?? 10).fill(1);
        this.pixelY = Array.from({length: this.rows.length + 1}, (_, i) => i);
        this.maxPixels = hooks.maxPixels ?? 32000000;
        this.fixedWidth = null;
      }
      async read(maximum) {geometryReads.push(maximum);}
      async calibrate() {}
      imageWidth() {return hooks.width ?? 3;}
      tiles(end, targets, cuts = hooks.tileRows ?? [[0, 3], [3, 6], [6, 9], [9, 10]], {allowGaps = false} = {}) {
        if (!allowGaps) {
          assert.equal(cuts[0][0], 0);
          assert.equal(cuts.at(-1)[1], this.rows.length);
        }
        return cuts.map(([first, last], index) => {
          if (index && !allowGaps) assert.equal(cuts[index - 1][1], first);
          assert.ok(last - first <= 3);
          const core = {r0: first, r1: last, c0: 0, c1: end + 1};
          return {core, raw: core, width: hooks.width ?? 3, expectedHeight: last - first, cropHeight: last - first, cropY: 0,
            pixelBudget: hooks.pixelBudget ?? this.maxPixels, budgetFallback: hooks.budgetFallback ?? null};
        });
      }
    }
  };
  globalThis.CaptureOverflow = {
    async crop(base64) {
      events.push('finalize');
      if (hooks.crop) return hooks.crop(base64);
      return base64;
    },
    async check(office) {
      if (hooks.probe) await office(hooks.probe);
      return {rightEdge: false};
    }
  };
  const helpers = {
    async office(action) {
      officeActive++;
      peakOfficeActive = Math.max(peakOfficeActive, officeActive);
      const before = captures.length;
      try {
        const result = await action({
          workbook: {
            name: workbookName,
            load() {},
            worksheets: {
              getItem() {
                return {
                  getRange(range) {
                    return {
                      getImage() {
                        captures.push(range);
                        events.push(`capture:${range}`);
                        const [first, last] = range.split(':').map(Number);
                        return {
                          value: hooks.encode ? hooks.encode(hooks.width ?? 3, last - first) : encode(hooks.width ?? 3, last - first)
                        };
                      }
                    };
                  }
                };
              }
            }
          },
          async sync() {
            if (hooks.sync) await hooks.sync(captures.slice(before));
          }
        });
        if (captures.length > before) captureBatches.push(captures.length - before);
        return result;
      } finally {
        officeActive--;
      }
    },
    bounds: sheet => ({bottom: sheet.bottom_row, right: 1}),
    async checkedWorksheet() {}, async hideGridlines() {}, async verifyDrawingBounds() {},
    async rowHeights(sheet) {heightSheets.push(sheet); return {sheet_index: sheet.index, runs: [[0, 10, 1]]};},
    phase() {}, progress() {}
  };
  const worker = createWorker(helpers, {async image(command, metadata, bytes) {
    uploads.push({command, metadata, bytes});
    events.push(`upload:${metadata.range}`);
    if (hooks.upload) await hooks.upload(command, metadata, bytes);
    return {image_path: `images/${command}/${metadata.tile_index}.png`, width: metadata.width,
      height: metadata.height, sha256: createHash('sha256').update(bytes).digest('hex')};
  }}, 'fixture.xlsx');
  const sheet = {index: 1, name: 'Sheet1', bottom_row: 9, row_height_bottom_row: 2,
    horizontal_overflow_required: false, drawing_fallback_required: false};
  return {worker, sheet, captures, uploads, heightSheets, geometryReads, captureBatches, events, change() {content = 'changed';},
    get peakOfficeActive() {return peakOfficeActive;},
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

test('readiness verifies the workbook without planning or allocating captures', async () => {
  const f = fixture();
  assert.equal(await f.worker.verifyWorkbook(), 'fixture.xlsx');
  assert.deepEqual(f.captures, []);
  assert.deepEqual(f.heightSheets, []);
  assert.equal(f.worker.resources().retained_capture_plans, 0);
  await assert.rejects(fixture('another.xlsx').worker.verifyWorkbook(), error => error.code === 'WORKBOOK_IDENTITY');
});

test('resource snapshots expose bounded image work and release retained geometry', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id:'plan',kind:'plan',sheets:[f.sheet]});
  await f.worker.execute({id:'image',kind:'capture_images',capture_id:'plan',tiles:plan.tiles.slice(0,2)});
  const before = f.worker.resources();
  assert.equal(before.retained_tiles, 4);
  assert.equal(before.geometry_rows, 10);
  assert.equal(before.pending_png_characters, 0);
  assert.equal(before.active_batch_pixels, 0);
  await f.worker.execute({id:'release',kind:'release',capture_id:'plan'});
  const after = f.worker.resources();
  assert.equal(after.retained_capture_plans, 0);
  assert.equal(after.retained_tiles, 0);
  assert.equal(after.geometry_rows, 0);
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
  const {SheetGeometry} = require('../web/capture-geometry.js');
  const geometry = new SheetGeometry(null, 'Sheet', 9, 1);
  geometry.rows = Array(10).fill(1);
  geometry.imageWidth = () => 3;
  for (const spans of [[], [[3, 6], [5, 8]], [[-1, 2]], [[0, 11]], [[1, 1]]]) {
    assert.throws(() => selectedTiles(geometry, 1, spans), /Invalid shared/);
  }
});

test('aggregate planning metadata is bounded across sheets before any captures', async () => {
  const tileRows = Array.from({length: 4096}, (_, index) => [index, index + 1]);
  const f = fixture('fixture.xlsx', {rowCount: 4096, tileRows});
  const sheets = Array.from({length: 33}, (_, index) => ({
    ...f.sheet, index: index + 1, name: `Sheet ${index + 1}`, bottom_row: 4095,
  }));
  await assert.rejects(f.worker.execute({id: 'large-plan', kind: 'plan', sheets}),
    error => error.code === 'IMAGE_BUDGET' && /metadata budget/.test(error.message));
  assert.equal(f.captures.length, 0);
  assert.equal(f.worker.resources().retained_capture_plans, 0);
  assert.equal(f.worker.resources().retained_tiles, 0);
  assert.ok(f.worker.resources().geometry_cache_entries <= 2);
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
    assert.equal(image.images[0].sha256, createHash('sha256').update(f.uploads[0].bytes).digest('hex'));
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

test('image-first rejects duplicate tiles or geometry that differs from its plan', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id:'plan',kind:'plan',sheets:[f.sheet]});
  await assert.rejects(f.worker.execute({id:'many',kind:'capture_images',capture_id:'plan',tiles:[plan.tiles[0],plan.tiles[0]]}),/differs from/);
  await assert.rejects(f.worker.execute({id:'wrong',kind:'capture_images',capture_id:'plan',tiles:[{...plan.tiles[0],width:99}]}),/differs from/);
  assert.deepEqual(f.captures,[]);
});

test('image-first batches render in one Office call and upload individual un-hashed tiles', async () => {
  const f = fixture();
  const digest = globalThis.crypto.subtle.digest;
  globalThis.crypto.subtle.digest = () => {throw new Error('Client hashing is forbidden');};
  try {
    const plan = await f.worker.execute({id:'plan',kind:'plan',sheets:[f.sheet],tile_megapixels:4});
    assert.deepEqual(plan.tiles.map(tile => tile.y), [0,3,6,9]);
    assert.deepEqual(plan.tiles.map(tile => tile.capture_pixels), [9,9,9,3]);
    const result = await f.worker.execute({id:'batch',kind:'capture_images',capture_id:'plan',tiles:plan.tiles});
    assert.deepEqual(f.captureBatches, [4]);
    assert.deepEqual(f.uploads.map(upload => upload.metadata.range), ['0:3','3:6','6:9','9:10']);
    assert.equal(result.uploaded_images,4);
    assert.equal(result.checksum_tiles,0);
    assert.ok(f.uploads.every(upload => upload.metadata.sha256 === undefined));
  } finally {globalThis.crypto.subtle.digest = digest;}
});

test('indivisible row budget is explicit in the plan and honored while capturing', async () => {
  const f = fixture('fixture.xlsx', {maxPixels: 5, pixelBudget: 32000000,
    budgetFallback: 'single-visible-row'});
  const plan = await f.worker.execute({id:'plan',kind:'plan',sheets:[f.sheet]});
  assert.equal(plan.tiles[0].pixel_budget, 32000000);
  assert.equal(plan.tiles[0].capture_budget_fallback, 'single-visible-row');
  assert.ok(plan.tiles[0].capture_pixels > 5);
  const result = await f.worker.execute({id:'capture',kind:'capture_images',capture_id:'plan',tiles:[plan.tiles[0]]});
  assert.equal(result.uploaded_images, 1);
});

test('ordinary captures cannot exceed their planned tile target', async () => {
  const f = fixture('fixture.xlsx', {maxPixels: 5});
  const plan = await f.worker.execute({id:'plan',kind:'plan',sheets:[f.sheet]});
  assert.equal(plan.tiles[0].pixel_budget, 5);
  assert.equal(plan.tiles[0].capture_budget_fallback, null);
  await assert.rejects(f.worker.execute({id:'capture',kind:'capture_images',capture_id:'plan',tiles:[plan.tiles[0]]}), /tile pixel budget/);
  assert.deepEqual(f.uploads, []);
});

test('image-first batches reject mixed sheets and unreasonable counts before Excel rendering', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id:'plan',kind:'plan',sheets:[f.sheet,{...f.sheet,index:2,name:'Sheet2'}]});
  await assert.rejects(f.worker.execute({id:'mixed',kind:'capture_images',capture_id:'plan',tiles:[plan.tiles[0],plan.tiles[4]]}), /same-sheet/);
  await assert.rejects(f.worker.execute({id:'empty',kind:'capture_images',capture_id:'plan',tiles:[]}), /1 to 16/);
  await assert.rejects(f.worker.execute({id:'many',kind:'capture_images',capture_id:'plan',tiles:Array(17).fill(plan.tiles[0])}), /1 to 16/);
  assert.deepEqual(f.captures,[]);
});

function deferred() {
  let resolve;
  const promise = new Promise(done => {
    resolve = done;
  });
  return {promise, resolve};
}

function assertReleased(worker) {
  const resources = worker.resources();
  assert.equal(resources.pending_png_characters, 0);
  assert.equal(resources.pending_encoded_bytes, 0);
  assert.equal(resources.active_batch_pixels, 0);
  assert.equal(resources.active_batch_tiles, 0);
}

test('bounded chunks overlap upload with the next capture but keep Office probes serial', async () => {
  const uploadStarted = deferred();
  const releaseUpload = deferred();
  const secondCapture = deferred();
  const f = fixture('fixture.xlsx', {
    async upload(command, metadata) {
      if (metadata.tile_index === 0) {
        uploadStarted.resolve();
        await releaseUpload.promise;
      }
    },
    async sync(ranges) {
      if (ranges.includes('6:9')) secondCapture.resolve();
    },
    async probe() {
      f.events.push('probe');
      await Promise.resolve();
    }
  });
  const plan = await f.worker.execute({
    id: 'plan',
    kind: 'plan',
    sheets: [{...f.sheet, horizontal_overflow_required: true}]
  });
  const execution = f.worker.execute({
    id: 'pipeline',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles,
    capture_batches: [2, 4]
  });
  await uploadStarted.promise;
  await secondCapture.promise;
  try {
    assert.equal(f.uploads.length, 1);
    assert.ok(f.worker.resources().pending_encoded_bytes > 0);
    assert.ok(f.events.indexOf('probe') < f.events.indexOf('capture:6:9'));
    const beforeSecondCapture = f.events.slice(0, f.events.indexOf('capture:6:9'));
    assert.equal(beforeSecondCapture.filter(event => event === 'probe').length, 2);
  } finally {
    releaseUpload.resolve();
  }
  const result = await execution;
  assert.deepEqual(f.captureBatches, [2, 2]);
  assert.equal(f.peakOfficeActive, 1);
  assert.equal(result.timings.pipeline_overlap_count, 1);
  assert.ok(result.timings.peak_pending_encoded_bytes > 0);
  assert.deepEqual(result.images.map(image => image.tile_index), [0, 1, 2, 3]);
  assertReleased(f.worker);
});

test('pipeline preserves the sequential image bytes and receipt order', async () => {
  const reference = fixture();
  const plan = await reference.worker.execute({id: 'plan', kind: 'plan', sheets: [reference.sheet]});
  const command = {
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles
  };
  const sequential = await reference.worker.execute(command);
  assert.deepEqual(reference.captureBatches, [4]);
  assert.deepEqual(reference.events.slice(4), [
    'finalize', 'upload:0:3', 'finalize', 'upload:3:6',
    'finalize', 'upload:6:9', 'finalize', 'upload:9:10'
  ]);
  assert.equal(sequential.timings.pipeline_overlap_count, 0);

  const pipeline = fixture();
  await pipeline.worker.execute({id: 'plan', kind: 'plan', sheets: [pipeline.sheet]});
  const overlapped = await pipeline.worker.execute({...command, capture_batches: [2, 4]});
  assert.deepEqual(overlapped.images, sequential.images);
  assert.deepEqual(pipeline.uploads, reference.uploads);
  assertReleased(reference.worker);
  assertReleased(pipeline.worker);
});

test('a single explicit chunk completes without looking for a second chunk', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const result = await f.worker.execute({
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles.slice(0, 2),
    capture_batches: [2]
  });
  assert.deepEqual(f.captureBatches, [2]);
  assert.equal(result.images.length, 2);
  assert.equal(result.timings.pipeline_overlap_count, 0);
  assertReleased(f.worker);
});

test('pipeline rejects malformed chunk boundaries before capturing', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const invalidBoundaries = [null, [], '4', [0, 4], [2, 2], [3, 2], [1, 2, 4], [2, 3], [1.5, 4], [5, 4]];
  for (const capture_batches of invalidBoundaries) {
    await assert.rejects(f.worker.execute({
      id: 'images',
      kind: 'capture_images',
      capture_id: 'plan',
      tiles: plan.tiles,
      capture_batches
    }), /batch boundaries/);
  }
  assert.deepEqual(f.captures, []);
  assertReleased(f.worker);
});

test('an upload failure stops the pipeline and releases both chunks', async () => {
  const f = fixture('fixture.xlsx', {
    async upload() {
      throw new Error('Host rejected the first image');
    }
  });
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  await assert.rejects(f.worker.execute({
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles,
    capture_batches: [2, 4]
  }), /Host rejected/);
  assert.equal(f.uploads.length, 1);
  assertReleased(f.worker);
});

test('failure in the second capture drains the in-flight upload before returning', async () => {
  const releaseUpload = deferred();
  const failedCapture = deferred();
  const f = fixture('fixture.xlsx', {
    async upload() {
      await releaseUpload.promise;
    },
    async sync(ranges) {
      if (ranges.includes('6:9')) {
        failedCapture.resolve();
        throw new Error('Excel capture failed');
      }
    }
  });
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  let settled = false;
  const execution = f.worker.execute({
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles,
    capture_batches: [2, 4]
  }).finally(() => {
    settled = true;
  });
  const rejection = assert.rejects(execution, /Excel capture failed/);
  await failedCapture.promise;
  await new Promise(resolve => setImmediate(resolve));
  try {
    assert.equal(settled, false);
  } finally {
    releaseUpload.resolve();
  }
  await rejection;
  assert.equal(f.uploads.length, 1);
  assertReleased(f.worker);
});

test('encoded-memory limit rejects oversized captures before decoding or uploading', async () => {
  // The capture mock reports a large encoded result without allocating it.
  // The guard must reject it before the PNG parser or decoder can inspect it.
  const f = fixture('fixture.xlsx', {encode: () => ({length: 100 * 1024 * 1024})});
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  await assert.rejects(f.worker.execute({
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles.slice(0, 1),
    capture_batches: [1]
  }), error => error.code === 'IMAGE_BUDGET');
  assert.deepEqual(f.uploads, []);
  assertReleased(f.worker);
});

test('encoded-memory limit includes the earlier chunk while its upload is pending', async () => {
  const releaseUpload = deferred();
  const secondCapture = deferred();
  let captures = 0;
  const f = fixture('fixture.xlsx', {
    encode(width, height) {
      captures++;
      if (captures === 2) {
        secondCapture.resolve();
        // This chunk alone fits exactly. Retaining the earlier binary makes
        // the combined work exceed the limit.
        return {length: 80 * 1024 * 1024};
      }
      return Buffer.from(JSON.stringify({width, height})).toString('base64');
    },
    async upload() {
      await releaseUpload.promise;
    }
  });
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const execution = f.worker.execute({
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles.slice(0, 2),
    capture_batches: [1, 2]
  });
  const rejection = assert.rejects(execution, error => error.code === 'IMAGE_BUDGET');
  await secondCapture.promise;
  await new Promise(resolve => setImmediate(resolve));
  try {
    assert.equal(f.uploads.length, 1);
    assert.ok(f.worker.resources().pending_encoded_bytes > 0);
  } finally {
    releaseUpload.resolve();
  }
  await rejection;
  assertReleased(f.worker);
});

test('finalization budgets cropped strings and decoding buffers before allocating bytes', async () => {
  const cropped = {length: 40 * 1024 * 1024};
  const f = fixture('fixture.xlsx', {crop: () => cropped});
  const pngSize = CaptureGeometry.pngSize;
  CaptureGeometry.pngSize = value => value === cropped ? {width: 3, height: 3} : pngSize(value);
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  await assert.rejects(f.worker.execute({
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles.slice(0, 1),
    capture_batches: [1]
  }), error => error.code === 'IMAGE_BUDGET');
  assert.deepEqual(f.uploads, []);
  assertReleased(f.worker);
});

test('decoding failure releases reserved temporary bytes and the captured input', async () => {
  const f = fixture('fixture.xlsx', {crop: () => 'invalid base64!'});
  const pngSize = CaptureGeometry.pngSize;
  CaptureGeometry.pngSize = value => value === 'invalid base64!' ? {width: 3, height: 3} : pngSize(value);
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  await assert.rejects(f.worker.execute({
    id: 'images',
    kind: 'capture_images',
    capture_id: 'plan',
    tiles: plan.tiles.slice(0, 1),
    capture_batches: [1]
  }), error => error.name === 'InvalidCharacterError');
  assert.deepEqual(f.uploads, []);
  assertReleased(f.worker);
});


test('hash-first retains exact PNGs and uploads them without another Excel capture', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const result = await f.worker.execute({
    id: 'hashes', kind: 'capture_hashes', capture_id: 'plan', tiles: plan.tiles.slice(0, 2)
  });
  assert.equal(result.checksum_tiles, 2);
  assert.equal(result.uploaded_images, 0);
  assert.ok(result.timings.hash_ms >= 0);
  assert.deepEqual(f.uploads, []);
  const resources = f.worker.resources();
  assert.equal(resources.retained_image_tiles, 2);
  assert.equal(resources.retained_image_bytes, result.tiles.reduce((sum, tile) => sum + tile.png_bytes, 0));
  assert.equal(resources.pending_encoded_bytes, resources.retained_image_bytes);

  f.change();
  const uploaded = await f.worker.execute({
    id: 'upload', kind: 'upload_retained', capture_id: 'plan', tiles: result.tiles
  });
  assert.deepEqual(f.captures, ['0:3', '3:6']);
  assert.equal(uploaded.uploaded_images, 2);
  assert.equal(f.worker.resources().retained_image_tiles, 2);
  for (let index = 0; index < f.uploads.length; index++) {
    assert.equal(createHash('sha256').update(f.uploads[index].bytes).digest('hex'), result.tiles[index].sha256);
    assert.equal(JSON.parse(Buffer.from(f.uploads[index].bytes)).content, 'stable');
  }
  const discard = {id: 'discard', kind: 'discard_retained', capture_id: 'plan', tiles: [result.tiles[0]]};
  await f.worker.execute(discard);
  await f.worker.execute({...discard, id: 'discard-again'});
  assert.equal(f.worker.resources().retained_image_tiles, 1);
  await f.worker.execute({id: 'release', kind: 'release', capture_id: 'plan'});
  assert.equal(f.worker.resources().retained_image_tiles, 0);
  assertReleased(f.worker);
});

test('retained images are isolated by capture plan and reject forged metadata', async () => {
  const f = fixture();
  const snapshots = [];
  for (const capture_id of ['first', 'second']) {
    const plan = await f.worker.execute({id: capture_id, kind: 'plan', sheets: [f.sheet]});
    snapshots.push(await f.worker.execute({
      id: `${capture_id}-hash`, kind: 'capture_hashes', capture_id, tiles: [plan.tiles[0]]
    }));
  }
  const request = {id: 'upload', kind: 'upload_retained', capture_id: 'first', tiles: snapshots[0].tiles};
  for (const change of [{sha256: '0'.repeat(64)}, {width: 1}, {range: 'wrong'}]) {
    await assert.rejects(f.worker.execute({...request, tiles: [{...request.tiles[0], ...change}]}), /no longer matches/);
  }
  await assert.rejects(f.worker.execute({...request, tiles: [...request.tiles, ...request.tiles]}), /no longer matches/);
  await f.worker.execute({id: 'release', kind: 'release', capture_id: 'first'});
  await assert.rejects(f.worker.execute(request), /Invalid retained-image request/);
  assert.deepEqual(f.uploads, []);
  await f.worker.execute({...request, capture_id: 'second', tiles: snapshots[1].tiles});
  assert.equal(f.uploads.length, 1);
  assert.equal(f.worker.resources().retained_image_tiles, 1);
  await f.worker.execute({id: 'release-all', kind: 'release'});
  assertReleased(f.worker);
});

test('retained tiles cannot silently be replaced by a recapture', async () => {
  const f = fixture();
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const command = {id: 'hash', kind: 'capture_hashes', capture_id: 'plan', tiles: [plan.tiles[0]]};
  await f.worker.execute(command);
  f.change();
  await assert.rejects(f.worker.execute({...command, id: 'again'}), /already retained/);
  assert.deepEqual(f.captures, ['0:3']);
  assert.equal(f.worker.resources().retained_image_tiles, 1);
});

test('failed checksum batches release their new images and preserve earlier context', async () => {
  let finalized = 0;
  const f = fixture('fixture.xlsx', {async crop(base64) {
    if (++finalized === 3) throw new Error('Crop failed');
    return base64;
  }});
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const previous = await f.worker.execute({
    id: 'first', kind: 'capture_hashes', capture_id: 'plan', tiles: [plan.tiles[0]]
  });
  await assert.rejects(f.worker.execute({
    id: 'next', kind: 'capture_hashes', capture_id: 'plan', tiles: plan.tiles.slice(1, 3)
  }), /Crop failed/);
  assert.equal(f.worker.resources().retained_image_tiles, 1);
  assert.equal(f.worker.resources().pending_encoded_bytes, previous.tiles[0].png_bytes);
  await f.worker.execute({id: 'context', kind: 'upload_retained', capture_id: 'plan', tiles: previous.tiles});
  assert.equal(f.uploads.length, 1);
});

test('retained tile credits cover all capture plans and are returned explicitly', async () => {
  const f = fixture();
  let lastPlan;
  for (let index = 0; index < 5; index++) {
    const id = `plan-${index}`;
    const plan = await f.worker.execute({id, kind: 'plan', sheets: [f.sheet]});
    lastPlan = {id: `hash-${index}`, kind: 'capture_hashes', capture_id: id, tiles: plan.tiles};
    if (index < 4) await f.worker.execute(lastPlan);
  }
  await assert.rejects(f.worker.execute(lastPlan), error => error.code === 'IMAGE_BUDGET');
  assert.equal(f.captures.length, 16);
  assert.equal(f.worker.resources().retained_image_tiles, 16);
  await f.worker.execute({id: 'release', kind: 'release', capture_id: 'plan-0'});
  await f.worker.execute(lastPlan);
  assert.equal(f.worker.resources().retained_image_tiles, 16);
  await f.worker.execute({id: 'all', kind: 'release'});
  assertReleased(f.worker);
});

test('retained pixels count against the next capture budget', async () => {
  // Mock dimensions exercise the pixel ledger without allocating decoded images.
  const f = fixture('fixture.xlsx', {width: 7000000});
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  await f.worker.execute({id: 'first', kind: 'capture_hashes', capture_id: 'plan', tiles: plan.tiles.slice(0, 2)});
  await assert.rejects(f.worker.execute({
    id: 'next', kind: 'capture_hashes', capture_id: 'plan', tiles: plan.tiles.slice(2)
  }), error => error.code === 'IMAGE_BUDGET');
  assert.deepEqual(f.captures, ['0:3', '3:6']);
  assert.equal(f.worker.resources().retained_image_tiles, 2);
});

test('encoded-memory guard includes PNGs retained by a previous command', async () => {
  let captures = 0;
  const f = fixture('fixture.xlsx', {encode(width, height) {
    if (++captures === 2) return {length: 80 * 1024 * 1024};
    return Buffer.from(JSON.stringify({width, height})).toString('base64');
  }});
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const first = await f.worker.execute({id: 'first', kind: 'capture_hashes', capture_id: 'plan', tiles: [plan.tiles[0]]});
  await assert.rejects(f.worker.execute({
    id: 'next', kind: 'capture_hashes', capture_id: 'plan', tiles: [plan.tiles[1]]
  }), error => error.code === 'IMAGE_BUDGET');
  assert.equal(f.worker.resources().retained_image_tiles, 1);
  assert.equal(f.worker.resources().pending_encoded_bytes, first.tiles[0].png_bytes);
  assert.equal(f.worker.resources().pending_png_characters, 0);
});

test('a failed retained upload keeps its original bytes available until release', async () => {
  let rejectUpload = true;
  const f = fixture('fixture.xlsx', {async upload() {
    if (rejectUpload) throw new Error('Upload failed');
  }});
  const plan = await f.worker.execute({id: 'plan', kind: 'plan', sheets: [f.sheet]});
  const result = await f.worker.execute({id: 'hash', kind: 'capture_hashes', capture_id: 'plan', tiles: [plan.tiles[0]]});
  const upload = {id: 'upload', kind: 'upload_retained', capture_id: 'plan', tiles: result.tiles};
  await assert.rejects(f.worker.execute(upload), /Upload failed/);
  assert.equal(f.worker.resources().retained_image_tiles, 1);
  rejectUpload = false;
  f.change();
  await f.worker.execute({...upload, id: 'retry'});
  assert.deepEqual(f.captures, ['0:3']);
  assert.deepEqual(f.uploads[0].bytes, f.uploads[1].bytes);
  await f.worker.execute({id: 'discard', kind: 'discard_retained', capture_id: 'plan', tiles: result.tiles});
  assertReleased(f.worker);
});
