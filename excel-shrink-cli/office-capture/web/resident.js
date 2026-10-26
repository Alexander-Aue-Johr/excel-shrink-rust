/* global CaptureGeometry, CaptureOverflow, ResidentConnection */
(function (root) {
  'use strict';

  const key = tile => `${tile.sheet_index}:${tile.tile_index}`;
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));

  async function encoded(base64, hash = true) {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index);
    if (!hash) return {bytes};
    const digest = new Uint8Array(await root.crypto.subtle.digest('SHA-256', bytes));
    const sha256 = Array.from(digest, value => value.toString(16).padStart(2, '0')).join('');
    return {bytes, sha256};
  }

  // The geometry helper verifies complete coverage. Fill unrequested gaps for
  // that check, then discard them before asking Excel for any images.
  function selectedTiles(geometry, end, requested) {
    if (requested === undefined) return geometry.tiles(end, null);
    if (!Array.isArray(requested) || !requested.length || requested.length > 4096) {
      throw new Error('Invalid requested tile rows');
    }
    const full = [], wanted = new Set();
    let cursor = 0;
    for (const span of requested) {
      if (!Array.isArray(span) || span.length !== 2 || !span.every(Number.isInteger) ||
          span[0] < cursor || span[0] >= span[1] || span[1] > geometry.rows.length) {
        throw new Error('Invalid requested tile rows');
      }
      // Unrequested spans must also fit the pixel budget. Use the normal plan
      // to split them; splitting a gap never changes the requested boundaries.
      if (cursor < span[0]) full.push(...gapCuts(geometry, end, cursor, span[0]));
      full.push(span);
      wanted.add(`${span[0]}:${span[1]}`);
      cursor = span[1];
    }
    if (cursor < geometry.rows.length) full.push(...gapCuts(geometry, end, cursor, geometry.rows.length));
    return geometry.tiles(end, null, full).filter(tile => wanted.has(`${tile.core.r0}:${tile.core.r1}`));
  }

  function gapCuts(geometry, end, first, last) {
    const cuts = geometry.tiles(end, null).map(tile => [tile.core.r0, tile.core.r1]);
    return cuts.filter(span => span[0] < last && span[1] > first)
      .map(span => [Math.max(first, span[0]), Math.min(last, span[1])]);
  }

  function createWorker(helpers, transport, expectedWorkbook) {
    const {office, bounds, checkedWorksheet, rowHeights, hideGridlines, verifyDrawingBounds} = helpers;
    const captures = new Map();
    const geometries = new Map();
    let workbookVerified = false;
    let pendingPngCharacters = 0, activeBatchPixels = 0, activeBatchTiles = 0;
    let timings;
    let lastCommandTiming;

    async function measure(field, action) {
      const started = performance.now();
      try {
        return await action();
      } finally {
        if (timings) timings[field] += performance.now() - started;
      }
    }

    function resources() {
      const sample = {
        retained_capture_plans: captures.size,
        retained_tiles: [...captures.values()].reduce((sum, plan) => sum + plan.size, 0),
        geometry_cache_entries: geometries.size,
        geometry_rows: [...geometries.values()].reduce((sum, geometry) => sum + geometry.rows.length, 0),
        pending_png_characters: pendingPngCharacters,
        active_batch_pixels: activeBatchPixels, active_batch_tiles: activeBatchTiles,
        script_evaluations: root.__excelShrinkPaneRuntime?.evaluations ?? 0
      };
      if (lastCommandTiming) sample.last_command_timing = {...lastCommandTiming};
      // Chromium may expose approximate JS heap figures. They do not include
      // decoded/native images; launcher process snapshots cover that separately.
      if (performance.memory) {
        sample.js_heap_used_bytes = performance.memory.usedJSHeapSize;
        sample.js_heap_limit_bytes = performance.memory.jsHeapSizeLimit;
      }
      return sample;
    }

    function rememberGeometry(id, geometry) {
      geometries.delete(id);
      geometries.set(id, geometry);
      // A million-row geometry has several large numeric arrays. Waiting for
      // other variants must not retain those arrays for every worksheet.
      while (geometries.size > 2) geometries.delete(geometries.keys().next().value);
    }

    async function geometryFor(state) {
      let geometry = geometries.get(state.geometry_key);
      if (!geometry) {
        geometry = new CaptureGeometry.SheetGeometry(office, state.sheet.name,
          state.sheet.bottom_row, state.right, state.megapixels);
        await geometry.read(state.end);
        await geometry.calibrate();
        if (state.probe && geometry.fixedWidth !== null) throw new Error(`Cannot check fixed-width sheet ${state.sheet.name}`);
        await verifyDrawingBounds(state.sheet, geometry, state.end);
      }
      rememberGeometry(state.geometry_key, geometry);
      return geometry;
    }

    async function finishImage(state, tile, base64, hash = true) {
      const {sheet, probe, right, end} = state;
      const geometry = await geometryFor(state);
      const dimensions = CaptureGeometry.pngSize(base64);
      if (dimensions.width * dimensions.height > geometry.maxPixels) {
        const error = new Error('Image exceeds the tile pixel budget');
        error.code = 'IMAGE_BUDGET';
        throw error;
      }
      if (dimensions.width !== tile.width || dimensions.height !== tile.expectedHeight) {
        throw new Error(`Image dimensions changed on ${sheet.name}`);
      }
      const overflow = probe
        ? await CaptureOverflow.check(office, sheet.name, geometry, tile, right, base64)
        : {rightEdge: false};
      if (overflow.rightEdge) throw new Error(`Text reaches the unverified right edge on ${sheet.name}`);
      const final = await CaptureOverflow.crop(base64, 0, tile.cropY, geometry.imageWidth(end), tile.cropHeight);
      const size = CaptureGeometry.pngSize(final);
      if (size.width !== geometry.imageWidth(end) || size.height !== tile.cropHeight) {
        throw new Error('Image crop changed dimensions');
      }
      return {...await encoded(final, hash), ...size};
    }

    async function render(state, tile, hash = true) {
      await geometryFor(state);
      const base64 = await measure('capture_ms', () => office(async context => {
        const image = context.workbook.worksheets.getItem(state.sheet.name)
          .getRange(CaptureGeometry.address(tile.raw)).getImage();
        await context.sync();
        return image.value;
      }));
      return measure('finish_image_ms', () => finishImage(state, tile, base64, hash));
    }

    async function verifyWorkbook() {
      if (!workbookVerified) {
        await office(async context => {
          context.workbook.load('name');
          await context.sync();
          if (typeof expectedWorkbook !== 'string' || !expectedWorkbook.length || context.workbook.name !== expectedWorkbook) {
            const error = new Error(`Workbook identity differs: expected ${JSON.stringify(expectedWorkbook)}, received ${JSON.stringify(context.workbook.name)}`);
            error.code = 'WORKBOOK_IDENTITY';
            throw error;
          }
        });
        workbookVerified = true;
      }
      return expectedWorkbook;
    }

    async function capture(command) {
      if (!Array.isArray(command.sheets) || !command.sheets.length ||
          (command.tile_megapixels !== undefined && (!Number.isInteger(command.tile_megapixels) ||
            command.tile_megapixels < 1 || command.tile_megapixels > 32))) {
        throw new Error('Incomplete resident capture command');
      }
      await verifyWorkbook();
      const result = {heights: [], sheets: [], tiles: []};
      if (captures.has(command.id)) throw new Error('Capture command was already executed');
      const retained = new Map();
      for (const sheet of command.sheets) {
        helpers.phase(command.kind === 'plan' ? 'Planning capture tiles' : 'Capturing tile checksums');
        helpers.progress(result.sheets.length, command.sheets.length, sheet.name);
        const {bottom, right} = bounds(sheet, true);
        await checkedWorksheet(sheet);
        result.heights.push(await rowHeights(sheet));
        await hideGridlines(sheet);
        const geometry = new CaptureGeometry.SheetGeometry(office, sheet.name, bottom, right,
          command.tile_megapixels ?? 32);
        const probe = sheet.horizontal_overflow_required && right < 16383;
        const maximum = sheet.capture_right_column ?? (probe ? Math.min(16383, right + 256) : right);
        if (!Number.isInteger(maximum) || maximum < right || maximum >= 16384 ||
            (probe && maximum === right)) throw new Error('Invalid maximum capture boundary');
        // Read enough geometry to negotiate a wider peer boundary, but capture
        // only the verified local extent until the orchestrator requests more.
        await geometry.read(maximum);
        await geometry.calibrate();
        if (probe && geometry.fixedWidth !== null) throw new Error(`Cannot check fixed-width sheet ${sheet.name}`);
        const proposed = probe ? CaptureGeometry.probeEnd(geometry, right) : right;
        const end = sheet.capture_end_column ?? proposed;
        if (!Number.isInteger(end) || end < proposed || end > maximum) {
          throw new Error('Negotiated capture boundary is outside the verified extent');
        }
        const tiles = selectedTiles(geometry, end, sheet.tile_rows);
        await verifyDrawingBounds(sheet, geometry, end);
        const state = {sheet, probe, right, end, geometry_key: `${command.id}:${sheet.index}`,
          megapixels: command.tile_megapixels ?? 32};
        rememberGeometry(state.geometry_key, geometry);
        result.sheets.push({sheet_index: sheet.index, sheet_name: sheet.name,
          capture_right_column: end, tile_rows: tiles.map(tile => [tile.core.r0, tile.core.r1])});
        for (let index = 0; index < tiles.length; index++) {
          const tile = tiles[index];
          const image = command.kind === 'plan'
            ? {width: geometry.imageWidth(end), height: tile.cropHeight, sha256: '0'.repeat(64)}
            : await render(state, tile);
          const metadata = {sheet_index: sheet.index, sheet_name: sheet.name, tile_index: index,
            range: CaptureGeometry.address(tile.core), core_rows: [tile.core.r0, tile.core.r1],
            x: 0, y: geometry.pixelY[tile.core.r0],
            capture_pixels: tile.width * tile.expectedHeight,
            capture_right_column: end, width: image.width, height: image.height, sha256: image.sha256};
          result.tiles.push(metadata);
          retained.set(key(metadata), {...state, tile, metadata});
          // No encoded or decoded images are retained while waiting for peers.
        }
      }
      captures.set(command.id, retained);
      helpers.progress(command.sheets.length, command.sheets.length);
      return result;
    }

    async function images(command) {
      if (!Array.isArray(command.tiles) || !command.tiles.length) throw new Error('No requested evidence tiles');
      const retained = captures.get(command.capture_id);
      if (!retained) throw new Error('Unknown capture command');
      const result = {images: []};
      const visited = new Set();
      for (const requested of command.tiles) {
        const id = key(requested), state = retained.get(id);
        if (!state || visited.has(id)) throw new Error('Unknown or duplicate evidence tile');
        visited.add(id);
        const expected = state.metadata;
        if (requested.sha256 !== expected.sha256 || requested.width !== expected.width || requested.height !== expected.height) {
          throw new Error('Evidence request no longer matches offered tile');
        }
        helpers.phase('Capturing differing tile');
        await checkedWorksheet(state.sheet);
        const image = await render(state, state.tile);
        if (image.sha256 !== expected.sha256 || image.width !== expected.width || image.height !== expected.height) {
          const error = new Error(`Worksheet image changed after checksum capture: ${state.sheet.name}!${expected.range}`);
          error.code = 'CAPTURE_CHANGED';
          throw error;
        }
        const receipt = await measure('upload_ms', () => transport.image(command.id, expected, image.bytes));
        if (typeof receipt.image_path !== 'string' || !receipt.image_path) throw new Error('Missing image receipt');
        result.images.push({...expected, image_path: receipt.image_path});
      }
      return result;
    }

    // The host requests a bounded set after every peer has offered its geometry.
    // No digest is calculated in this path, including on evidence recaptures.
    async function captureImages(command) {
      if (!Array.isArray(command.tiles) || !command.tiles.length || command.tiles.length > 16) {
        throw new Error('Image-first jobs require 1 to 16 tiles');
      }
      const retained = captures.get(command.capture_id);
      if (!retained) throw new Error('Unknown capture plan');
      const seen = new Set();
      const states = command.tiles.map(requested => {
        const id = key(requested), state = retained.get(id);
        if (!state || seen.has(id) || requested.range !== state.metadata.range ||
            requested.width !== state.metadata.width || requested.height !== state.metadata.height) {
          throw new Error('Image request differs from the capture plan');
        }
        seen.add(id);
        return state;
      });
      if (states.some(state => state.geometry_key !== states[0].geometry_key) ||
          states.reduce((sum, state) => sum + state.tile.width * state.tile.expectedHeight, 0) > 64000000) {
        throw new Error('Image-first batch exceeds the same-sheet 64 megapixel budget');
      }
      helpers.phase(`Capturing synchronized batch (${states.length} tiles)`);
      await checkedWorksheet(states[0].sheet);
      await geometryFor(states[0]);
      // One Office request captures the bounded batch. Upload each finished PNG
      // immediately; the host does not wait for the final batch result to paint it.
      activeBatchTiles = states.length;
      activeBatchPixels = states.reduce((sum, state) => sum + state.tile.width * state.tile.expectedHeight, 0);
      const base64s = await measure('capture_ms', () => office(async context => {
        const worksheet = context.workbook.worksheets.getItem(states[0].sheet.name);
        const images = states.map(state => worksheet.getRange(CaptureGeometry.address(state.tile.raw)).getImage());
        await context.sync();
        return images.map(image => image.value);
      }));
      pendingPngCharacters = base64s.reduce((sum, value) => sum + value.length, 0);
      const result = {images: []};
      for (let index = 0; index < states.length; index++) {
        const state = states[index];
        const image = await measure('finish_image_ms', () => finishImage(state, state.tile, base64s[index], false));
        pendingPngCharacters -= base64s[index].length;
        base64s[index] = null;
        const receipt = await measure('upload_ms', () => transport.image(command.id,
          {...state.metadata, sha256: undefined}, image.bytes));
        if (typeof receipt.image_path !== 'string' || !receipt.image_path ||
            typeof receipt.sha256 !== 'string' || receipt.width !== image.width || receipt.height !== image.height) {
          throw new Error('Missing or invalid image-first receipt');
        }
        result.images.push({...state.metadata, ...receipt});
      }
      return result;
    }

    return {
      verifyWorkbook,
      resources,
      recordAcknowledgment(command, measured, acknowledgment) {
        lastCommandTiming = {command_id: command.id, kind: command.kind, ...measured, ...acknowledgment};
      },
      async execute(command) {
        const started = performance.now();
        timings = {capture_ms: 0, finish_image_ms: 0, upload_ms: 0};
        let result;
        try {
          if (command.kind === 'capture' || command.kind === 'plan') result = await capture(command);
          else if (command.kind === 'images') result = await images(command);
          else if (command.kind === 'capture_images') result = await captureImages(command);
          else if (command.kind === 'release') {
            if (command.capture_id) {
              captures.delete(command.capture_id);
              for (const id of geometries.keys()) {
                if (id.startsWith(command.capture_id + ':')) geometries.delete(id);
              }
            } else {
              captures.clear();
              geometries.clear();
            }
            result = {};
          } else throw new Error(`Unknown resident command: ${command.kind}`);
          result.elapsed_ms = performance.now() - started;
          result.timings = {...timings, execution_ms: result.elapsed_ms};
          lastCommandTiming = {command_id: command.id, kind: command.kind, ...result.timings};
          result.checksum_tiles = command.kind === 'capture' ? result.tiles?.length ?? 0 : 0;
          result.uploaded_images = result.images?.length ?? 0;
          return result;
        } finally {
          pendingPngCharacters = 0;
          activeBatchPixels = 0;
          activeBatchTiles = 0;
          timings = undefined;
        }
      }
    };
  }

  async function run(client, helpers) {
    let token;
    async function request(path, body, headers = {}, timeoutMs = 120000) {
      const abort = new AbortController();
      const timer = setTimeout(() => abort.abort(), timeoutMs);
      try {
        const response = await fetch(path, {cache: 'no-store', signal: abort.signal,
          method: body === undefined ? 'GET' : 'POST',
          headers: {...(token ? {'X-Client-Token': token} : {}), ...headers}, body});
        if (!response.ok) {
          const error = new Error(`${path}: HTTP ${response.status} ${(await response.text()).slice(0, 500)}`);
          error.status = response.status;
          throw error;
        }
        return await response.json();
      } finally {clearTimeout(timer);}
    }
    const clientPath = encodeURIComponent(client);
    const runtime = root.__excelShrinkPaneRuntime ||= {documentId: crypto.randomUUID(), evaluations: 0};
    const runId = crypto.randomUUID(), requestId = crypto.randomUUID();
    const config = await request(`/config?client=${clientPath}&document=${runtime.documentId}&run=${runId}&request=${requestId}`);
    token = config.client_token;
    if (typeof token !== 'string' || !token.length) throw new Error('Missing resident session token');
    helpers.show('workbook', config.workbook);
    helpers.connection(true, 'Resident worker connected');
    const worker = createWorker(helpers, {image(id, metadata, bytes) {
      return request(`/image/${clientPath}/${encodeURIComponent(id)}/${metadata.sheet_index}/${metadata.tile_index}`,
        bytes, {'Content-Type': 'image/png', ...(metadata.sha256 ? {'X-Sha256': metadata.sha256} : {})});
    }}, config.workbook);
    let heartbeatBusy = false;
    const heartbeat = setInterval(async () => {
      if (heartbeatBusy) return;
      heartbeatBusy = true;
      try {await request(`/heartbeat?client=${clientPath}`, JSON.stringify(worker.resources()),
        {'Content-Type':'application/json'});} catch (_) { /* Main job surfaces lost host. */ }
      finally {heartbeatBusy = false;}
    }, 5000);
    let jobs;
    try {
      helpers.phase('Verifying workbook identity');
      try {
        const workbook = await worker.verifyWorkbook();
        await request(`/ready?client=${clientPath}`, JSON.stringify({workbook}), {'Content-Type': 'application/json'});
      } catch (error) {
        try {await request(`/ready?client=${clientPath}`, JSON.stringify({ok: false,
          error: {code: error.code || 'ADDIN_INITIALIZATION_FAILED', message: error.message || String(error)}}),
          {'Content-Type': 'application/json'});} catch (_) { /* Preserve the original failure. */ }
        throw error;
      }
      const url = new URL('/events', location.href);
      url.protocol = location.protocol === 'https:' ? 'wss:' : 'ws:';
      jobs = ResidentConnection.createJobConnection({url: url.href, client, token,
        connection: helpers.connection});
      await ResidentConnection.runJobs({jobs, worker, sleep, phase: helpers.phase,
        sendResult(id, body, timeoutMs) {
          return request(`/result/${clientPath}/${encodeURIComponent(id)}`, body,
            {'Content-Type': 'application/json'}, timeoutMs);
        }});
    } finally {
      jobs?.close();
      clearInterval(heartbeat);
    }
  }

  const api = {run, createWorker, encoded, selectedTiles};
  root.ResidentCapture = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
