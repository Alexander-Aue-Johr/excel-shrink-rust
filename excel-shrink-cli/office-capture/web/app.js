/* global Office, Excel, CaptureGeometry, CaptureOverflow */
(function () {
  'use strict';

  // A repeated script evaluation in the same document must not start a second
  // polling loop. A genuinely new taskpane document receives a fresh identity.
  const runtime = window.__excelShrinkPaneRuntime ||= {
    documentId: crypto.randomUUID(), evaluations: 0, runStarted: false
  };
  runtime.evaluations++;
  if (runtime.evaluations > 1) return;

  const residentClient = new URLSearchParams(location.search).get('client');
  // Another Excel instance can serialize a workbook open across the desktop.
  // Resident sessions tolerate that pause; the launcher still bounds the job.
  const nativeTimeoutMs = residentClient ? 900000 : 90000;
  const requestTimeoutMs = 120000;
  const started = performance.now();
  let nativeStopped = false;
  let classicConfig;

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

  async function request(path, body, timeoutMs = requestTimeoutMs) {
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(), timeoutMs);
    try {
      const response = await fetch(path, body === undefined
        ? {signal: abort.signal, cache: 'no-store'}
        : {method: 'POST', headers: {'Content-Type': 'application/json'},
          body: JSON.stringify(body), signal: abort.signal});
      if (!response.ok) throw new Error(`${path}: HTTP ${response.status} ${(await response.text()).slice(0, 500)}`);
      return await response.json();
    } finally { clearTimeout(timer); }
  }

  async function classicObservation(state, details = {}, progress = false) {
    if (!classicConfig?.telemetry) return;
    try {
      const receipt = await request('/telemetry', {run_id: classicConfig.run_id, side: classicConfig.side || 'original',
        state, details, progress}, 2000);
      if (!receipt.telemetry_saved) throw new Error('Host could not save classic telemetry');
    } catch (error) {
      classicConfig.telemetry = false;
      console.warn('Classic observations disabled; capture continues', error);
    }
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
      if (!geometry.containsExtent(right, bottom, end)) {
        throw new Error(`Drawing extends beyond the capture range on ${sheet.name}`);
      }
    }
  }

  async function checkedWorksheet(sheet) {
    await office(async context => {
      const worksheet = context.workbook.worksheets.getItem(sheet.name);
      const active = context.workbook.worksheets.getActiveWorksheet();
      worksheet.load('name,position,visibility');
      active.load('name');
      await context.sync();
      if (worksheet.name !== sheet.name || worksheet.position + 1 !== sheet.index || worksheet.visibility !== 'Visible') {
        throw new Error(`Worksheet identity changed: ${JSON.stringify(sheet.name)}`);
      }
      if (active.name !== worksheet.name) {
        worksheet.activate();
        await context.sync();
      }
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

  async function captureSheet(config, sheet) {
    await classicObservation('planning_sheet', {sheet_index: sheet.index, sheet_name: sheet.name});
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
    const proposedEnd = probe ? CaptureGeometry.probeEnd(geometry, right) : right;
    let end = sharedEnd !== undefined ? sharedEnd : proposedEnd;
    // Plan rows at the widest allowed extent. A narrower negotiated image keeps
    // these row cuts, so both workbooks remain within the same pixel budget.
    let tiles = geometry.tiles(end, targets, sheet.tile_rows);
    if (config.mode === 'paired') {
      const shared = await request('/pair/geometry', {run_id: config.run_id, side: config.side,
        sheet_index: sheet.index, end_column: proposedEnd, max_end_column: end,
        tile_rows: tiles.map(tile => [tile.core.r0, tile.core.r1])});
      if (!Number.isInteger(shared.capture_right_column) ||
          shared.capture_right_column < proposedEnd || shared.capture_right_column > end) {
        throw new Error(`Invalid negotiated capture boundary for ${sheet.name}`);
      }
      end = shared.capture_right_column;
      tiles = geometry.tiles(end, targets, shared.tile_rows);
    }
    await verifyDrawingBounds(sheet, geometry, end);
    const captures = [];
    for (let tileIndex = 0; tileIndex < tiles.length; tileIndex++) {
      const tile = tiles[tileIndex];
      const details = {sheet_index: sheet.index, sheet_name: sheet.name, tile_index: tileIndex,
        range: CaptureGeometry.address(tile.core)};
      await classicObservation('capturing', details);
      const base64 = await office(async context => {
        const image = context.workbook.worksheets.getItem(sheet.name).getRange(CaptureGeometry.address(tile.raw)).getImage();
        await context.sync();
        return image.value;
      });
      const png = CaptureGeometry.pngSize(base64);
      if (png.width * png.height > (tile.pixelBudget ?? geometry.maxPixels)) {
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
      if (config.mode === 'paired') {
        await classicObservation('providing_images', details);
        await request('/pair/image', {run_id: config.run_id, side: config.side,
          sheet_index: sheet.index, sheet_name: sheet.name, range,
          tile_index: tileIndex, base64: finalPng});
        continue;
      }
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
    if (residentClient) {
      for (const source of ['/resident-connection.js', '/resident.js']) {
        await new Promise((resolve, reject) => {
          const script = document.createElement('script');
          script.src = source;
          script.onload = resolve;
          script.onerror = () => reject(new Error(`Cannot load resident capture script: ${source}`));
          document.head.appendChild(script);
        });
      }
      return window.ResidentCapture.run(residentClient, {
        office, bounds, checkedWorksheet, rowHeights, hideGridlines, verifyDrawingBounds,
        phase, progress, show, connection
      });
    }
    const side = new URLSearchParams(location.search).get('side');
    const config = await request('/config' + (side ? '?side=' + encodeURIComponent(side) : ''));
    if (!config || !['heights', 'capture', 'paired'].includes(config.mode) ||
        typeof config.run_id !== 'string' || !config.run_id.length ||
        typeof config.workbook !== 'string' || !config.workbook.length ||
        !Array.isArray(config.sheets) || !config.sheets.length ||
        (config.tile_megapixels !== undefined && ![16, 32].includes(config.tile_megapixels))) {
      if (config && typeof config.run_id === 'string' && config.run_id.length) {
        try { await request('/result', {run_id: config.run_id, side: config.side, heights: [], captures: [],
          error: {code: 'CAPTURE_FAILED', message: 'Capture configuration is incomplete'}}); } catch (_) { /* The host may be unavailable. */ }
      }
      throw new Error('Capture configuration is incomplete');
    }
    show('workbook', config.workbook);
    classicConfig = config;
    connection(true, 'Connected');
    const result = {run_id: config.run_id, side: config.side, heights: [], captures: []};
    const completed = [];
    progress(0, config.sheets.length);
    let heartbeatPending = false;
    const heartbeat = config.telemetry ? setInterval(async () => {
      if (heartbeatPending || !config.telemetry) return;
      heartbeatPending = true;
      try {
        const receipt = await request('/telemetry', {run_id: config.run_id, side: config.side || 'original', heartbeat: true}, 2000);
        if (!receipt.telemetry_saved) throw new Error('Host could not save classic telemetry');
      } catch (error) {
        // The work requests will report a lost host. Heartbeat failures do not
        // establish whether Excel's native call made progress.
        config.telemetry = false;
        console.warn('Classic heartbeat observations disabled; capture continues', error);
      } finally { heartbeatPending = false; }
    }, 2000) : null;
    try {
      for (let sheet of config.sheets) {
        progress(completed.length, config.sheets.length, `${sheet.index} · ${sheet.name}`);
        phase(config.mode === 'heights' ? 'Checking row heights' : 'Capturing worksheet');
        await classicObservation('preparing_sheet', {sheet_index: sheet.index, sheet_name: sheet.name});
        await checkedWorksheet(sheet);
        if (config.mode === 'paired' && config.check_heights) {
          phase('Checking row heights');
          await classicObservation('checking_row_heights', {sheet_index: sheet.index, sheet_name: sheet.name});
          const heights = await rowHeights(sheet);
          const decision = await request('/pair/heights', {run_id: config.run_id,
            side: config.side, sheet_index: sheet.index, heights});
          if (!Array.isArray(decision.rows)) throw new Error('Missing row-height comparison');
          sheet = {...sheet, rows: decision.rows.length ? decision.rows : undefined};
          phase(decision.rows.length ? 'Capturing changed rows' : 'Capturing worksheet');
        }
        if (config.mode === 'heights') result.heights.push(await rowHeights(sheet));
        else result.captures.push(...await captureSheet(config, sheet));
        if (config.mode === 'paired') {
          await request('/pair/sheet', {run_id: config.run_id, side: config.side, sheet_index: sheet.index});
        }
        completed.push(`${sheet.index} · ${sheet.name}`);
        show('completed-sheets', completed.join('\n'));
        progress(completed.length, config.sheets.length);
      }
      await classicObservation('finished', {}, true);
      await request('/result', result);
      phase('Complete');
      return;
    } catch (error) {
      await classicObservation('failed', {error: error.message});
      try { await request('/result', {run_id: config.run_id, side: config.side, heights: [], captures: [],
        error: {code: error && error.code ? error.code : 'CAPTURE_FAILED',
          message: error && error.message ? error.message : String(error)}}); } catch (_) { /* Host timeout is also a failure. */ }
      throw error;
    } finally {
      if (heartbeat !== null) clearInterval(heartbeat);
    }
  }

  Office.onReady(info => {
    if (runtime.runStarted) return;
    if (!info || info.host !== Office.HostType.Excel) {
      connection(false, 'Excel unavailable');
      phase('Open this add-in in Excel');
      return;
    }
    runtime.runStarted = true;
    run().catch(error => {
      connection(false, 'Capture failed');
      phase(error && error.message ? error.message : String(error));
    });
  });
}());
