/* global Office, Excel, CaptureGeometry, CaptureOverflow */
(function () {
  'use strict';

  const nativeTimeoutMs = 90000;
  const requestTimeoutMs = 120000;
  const started = performance.now();
  let nativeStopped = false;

  const element = id => document.getElementById(id);
  const show = (id, value) => { element(id).textContent = value; };
  function progress(done, total, sheet) {
    element('progress-bar').style.width = `${total ? done * 100 / total : 0}%`;
    element('progress').setAttribute('aria-valuenow', String(total ? Math.round(done * 100 / total) : 0));
    show('progress-text', `${done} / ${total} sheets`);
    show('elapsed', `${Math.round((performance.now() - started) / 1000)} s`);
    show('completed-count', done);
    if (sheet) show('current-sheet', sheet);
  }

  function phase(value) { show('phase', value); }
  function connection(ok, label) {
    element('light').className = `status-light ${ok ? 'good' : 'bad'}`;
    show('connection', label);
  }

  async function request(path, body) {
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), requestTimeoutMs);
    try {
      const response = await fetch(path, body === undefined
        ? {signal: abort.signal, cache: 'no-store'}
        : {method: 'POST', headers: {'Content-Type': 'application/json'},
          body: JSON.stringify(body), signal: abort.signal});
      if (!response.ok) throw new Error(`${path}: HTTP ${response.status} ${(await response.text()).slice(0, 500)}`);
      return await response.json();
    } finally { clearTimeout(timer); }
  }

  async function office(action) {
    if (nativeStopped) throw new Error('Excel stopped responding; no further workbook calls are safe');
    let timer;
    try {
      return await Promise.race([
        Excel.run(action),
        new Promise((_, reject) => {
          timer = setTimeout(() => {
            nativeStopped = true;
            reject(new Error('Excel operation timed out; the workbook copy must be discarded'));
          }, nativeTimeoutMs);
        })
      ]);
    } finally { clearTimeout(timer); }
  }

  function bounds(sheet, captureMode = false) {
    const bottom = sheet.bottom_row;
    const right = sheet.base_right_column ?? sheet.right_column;
    if (!Number.isInteger(sheet.index) || sheet.index < 1 ||
        typeof sheet.name !== 'string' || !sheet.name.length ||
        !Number.isInteger(bottom) || bottom < 0 || bottom >= 1048576 ||
        (captureMode && (!Number.isInteger(right) || right < 0 || right >= 16384 ||
          typeof sheet.horizontal_overflow_required !== 'boolean' ||
          typeof sheet.drawing_fallback_required !== 'boolean'))) {
      throw new Error(`Incomplete capture bounds for worksheet ${JSON.stringify(sheet.name)}`);
    }
    return {bottom, right};
  }

  async function verifyDrawingBounds(sheet, geometry, end) {
    if (!sheet.drawing_fallback_required) return;
    const shapes = await office(async context => {
      const collection = context.workbook.worksheets.getItem(sheet.name).shapes;
      collection.load('items/left,items/top,items/width,items/height,items/rotation,items/visible');
      await context.sync();
      return collection.items.map(shape => ({
        left: Number(shape.left), top: Number(shape.top),
        width: Number(shape.width), height: Number(shape.height),
        rotation: Number(shape.rotation), visible: shape.visible
      }));
    });
    if (!shapes.length) throw new Error(`Drawing boundary is unresolved for worksheet ${JSON.stringify(sheet.name)}`);
    const maxRight = geometry.x[end + 1];
    const maxBottom = geometry.y[sheet.bottom_row + 1];
    for (const shape of shapes) {
      if (typeof shape.visible !== 'boolean') throw new Error(`Invalid shape visibility on ${sheet.name}`);
      if (!shape.visible) continue;
      if (![shape.left, shape.top, shape.width, shape.height, shape.rotation].every(Number.isFinite) ||
          shape.width < 0 || shape.height < 0) throw new Error(`Invalid shape bounds on ${sheet.name}`);
      const angle = shape.rotation * Math.PI / 180;
      const rotatedWidth = Math.abs(shape.width * Math.cos(angle)) + Math.abs(shape.height * Math.sin(angle));
      const rotatedHeight = Math.abs(shape.width * Math.sin(angle)) + Math.abs(shape.height * Math.cos(angle));
      const right = shape.left + (shape.width + rotatedWidth) / 2;
      const bottom = shape.top + (shape.height + rotatedHeight) / 2;
      if (right > maxRight || bottom > maxBottom) {
        throw new Error(`Drawing extends beyond the capture range on ${sheet.name}`);
      }
    }
  }

  async function checkedWorksheet(sheet) {
    await office(async context => {
      const worksheet = context.workbook.worksheets.getItem(sheet.name);
      worksheet.load('name,position,visibility');
      await context.sync();
      if (worksheet.name !== sheet.name || worksheet.position + 1 !== sheet.index || worksheet.visibility !== 'Visible') {
        throw new Error(`Worksheet identity changed: ${JSON.stringify(sheet.name)}`);
      }
      worksheet.activate();
      await context.sync();
    });
  }

  async function rowHeights(sheet) {
    const {bottom} = bounds(sheet);
    const runs = await CaptureGeometry.readHeightRuns(office, sheet.name, bottom);
    return {sheet_index: sheet.index, sheet_name: sheet.name, runs};
  }

  async function hideGridlines(sheet) {
    await office(async context => {
      const worksheet = context.workbook.worksheets.getItem(sheet.name);
      worksheet.showGridlines = false;
      await context.sync();
      worksheet.load('showGridlines');
      await context.sync();
      if (worksheet.showGridlines !== false) throw new Error(`Cannot hide gridlines for ${sheet.name}`);
    });
  }

  function targetRows(sheet, bottom) {
    if (sheet.rows === undefined || sheet.rows === null) return null;
    if (!Array.isArray(sheet.rows) || !sheet.rows.length ||
        sheet.rows.some(row => !Number.isInteger(row) || row < 1 || row > bottom + 1)) {
      throw new Error(`Invalid changed rows for ${sheet.name}`);
    }
    return [...new Set(sheet.rows)].sort((a, b) => a - b);
  }

  function probeEnd(geometry, base, requested) {
    const origin = geometry.x[base + 1];
    let visible = 0, end = base;
    for (let column = base + 1; column < geometry.columns.length; column++) {
      if (geometry.columns[column] <= 0) continue;
      end = column;
      visible++;
      if (visible >= requested || (geometry.x[column + 1] - origin) * geometry.scaleX >= 1920) break;
    }
    if (end === base) throw new Error('No visible column is available for the overflow check');
    return end;
  }

  async function captureSheet(config, sheet) {
    const {bottom, right} = bounds(sheet, true);
    const targets = targetRows(sheet, bottom);
    await hideGridlines(sheet);
    const geometry = new CaptureGeometry.SheetGeometry(office, sheet.name, bottom, right,
      config.tile_megapixels ?? 32);
    const probe = sheet.horizontal_overflow_required && right < 16383;
    const sharedEnd = sheet.capture_right_column;
    if (sharedEnd !== undefined && (!Number.isInteger(sharedEnd) || sharedEnd < right || sharedEnd >= 16384 ||
        (probe && sharedEnd === right))) {
      throw new Error(`Invalid shared capture boundary for ${sheet.name}`);
    }
    const maxColumn = sharedEnd !== undefined ? sharedEnd : probe ? Math.min(16383, right + 256) : right;
    await geometry.read(maxColumn);
    await geometry.calibrate();
    if (probe && geometry.fixedWidth !== null) throw new Error(`Cannot check right-edge overflow on fixed-width sheet ${sheet.name}`);
    const end = sharedEnd !== undefined ? sharedEnd : probe ? probeEnd(geometry, right, 64) : right;
    await verifyDrawingBounds(sheet, geometry, end);
    const tiles = geometry.tiles(end, targets);
    const captures = [];
    for (let tileIndex = 0; tileIndex < tiles.length; tileIndex++) {
      const tile = tiles[tileIndex];
      const base64 = await office(async context => {
        const image = context.workbook.worksheets.getItem(sheet.name).getRange(CaptureGeometry.address(tile.raw)).getImage();
        await context.sync();
        return image.value;
      });
      const png = CaptureGeometry.pngSize(base64);
      if (png.width * png.height > geometry.maxPixels) {
        const error = new Error(`Image exceeds the tile budget (${png.width} × ${png.height})`);
        error.code = 'IMAGE_BUDGET';
        throw error;
      }
      if (png.width !== tile.width || png.height !== tile.expectedHeight) {
        throw new Error(`Image dimensions changed at ${sheet.name}!${CaptureGeometry.address(tile.raw)}: ` +
          `expected ${tile.width} × ${tile.expectedHeight}, received ${png.width} × ${png.height}`);
      }
      const result = probe
        ? await CaptureOverflow.check(office, sheet.name, geometry, tile, right, base64)
        : {retainProbe: false, rightEdge: false};
      if (result.rightEdge) throw new Error(`Text reaches the unverified right edge at ${sheet.name}!${CaptureGeometry.address(tile.raw)}`);
      const lastColumn = end;
      const width = geometry.imageWidth(end);
      const finalPng = await CaptureOverflow.crop(base64, 0, tile.cropY, width, tile.cropHeight);
      const finalSize = CaptureGeometry.pngSize(finalPng);
      if (finalSize.width !== width || finalSize.height !== tile.cropHeight) throw new Error('Image crop changed dimensions');
      const range = CaptureGeometry.address({...tile.core, c1: lastColumn + 1});
      const receipt = await request('/image', {run_id: config.run_id,
        sheet_index: sheet.index, sheet_name: sheet.name, range,
        tile_index: tileIndex, base64: finalPng});
      if (typeof receipt.path !== 'string' || !receipt.path.length) throw new Error('Capture host did not acknowledge the image');
      captures.push({sheet_index: sheet.index, sheet_name: sheet.name, range,
        tile_index: tileIndex, image_path: receipt.path});
    }
    return captures;
  }

  async function run() {
    const config = await request('/config');
    if (!config || !['heights', 'capture'].includes(config.mode) ||
        typeof config.run_id !== 'string' || !config.run_id.length ||
        typeof config.workbook !== 'string' || !config.workbook.length ||
        !Array.isArray(config.sheets) || !config.sheets.length ||
        (config.tile_megapixels !== undefined && ![16, 32].includes(config.tile_megapixels))) {
      if (config && typeof config.run_id === 'string' && config.run_id.length) {
        try { await request('/result', {run_id: config.run_id, heights: [], captures: [],
          error: {code: 'CAPTURE_FAILED', message: 'Capture configuration is incomplete'}}); } catch (_) { /* The host may be unavailable. */ }
      }
      throw new Error('Capture configuration is incomplete');
    }
    show('workbook', config.workbook);
    connection(true, 'Connected');
    const result = {run_id: config.run_id, heights: [], captures: []};
    const completed = [];
    progress(0, config.sheets.length);
    try {
      for (const sheet of config.sheets) {
        progress(completed.length, config.sheets.length, `${sheet.index} · ${sheet.name}`);
        phase(config.mode === 'heights' ? 'Checking row heights' : 'Capturing worksheet');
        await checkedWorksheet(sheet);
        if (config.mode === 'heights') result.heights.push(await rowHeights(sheet));
        else result.captures.push(...await captureSheet(config, sheet));
        completed.push(`${sheet.index} · ${sheet.name}`);
        show('completed-sheets', completed.join('\n'));
        progress(completed.length, config.sheets.length);
      }
      await request('/result', result);
      phase('Complete');
      return;
    } catch (error) {
      try { await request('/result', {run_id: config.run_id, heights: [], captures: [],
        error: {code: error && error.code ? error.code : 'CAPTURE_FAILED',
          message: error && error.message ? error.message : String(error)}}); } catch (_) { /* Host timeout is also a failure. */ }
      throw error;
    }
  }

  Office.onReady(info => {
    if (!info || info.host !== Office.HostType.Excel) {
      connection(false, 'Excel unavailable');
      phase('Open this add-in in Excel');
      return;
    }
    run().catch(error => {
      connection(false, 'Capture failed');
      phase(error && error.message ? error.message : String(error));
    });
  });
}());
