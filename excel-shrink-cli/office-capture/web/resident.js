/* global CaptureGeometry, CaptureOverflow, ResidentConnection */
(function (root) {
  'use strict';

  const key = tile => `${tile.sheet_index}:${tile.tile_index}`;
  const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
  const MAX_PLANNED_TILES_PER_COMMAND = 131072;

  async function encoded(base64, hash = true) {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index++) bytes[index] = binary.charCodeAt(index);
    if (!hash) return {bytes};
    const started = performance.now();
    const digest = new Uint8Array(await root.crypto.subtle.digest('SHA-256', bytes));
    const hashMs = performance.now() - started;
    const sha256 = Array.from(digest, value => value.toString(16).padStart(2, '0')).join('');
    return {bytes, sha256, hash_ms: hashMs};
  }

  function selectedTiles(geometry, end, requested, {planOnly = false} = {}) {
    // Validate and build only the selected row bands. Replanning omitted gaps
    // would hit the full-sheet budget again after tall-sheet selection.
    return geometry.tiles(end, null, requested, {planOnly, allowGaps: requested !== undefined});
  }

  /**
   * Count completed sheets in the entire process sweep, including both hash passes.
   * completedSheets is the count within this command; sheet supplies the current label.
   */
  function sheetSweepProgress(command, completedSheets, sheet) {
    const pass = command.progress_pass ?? 1;
    const passes = command.progress_passes ?? 1;
    const sheetCount = command.sheets.length;
    return {
      command_id: command.id, sheet_name: sheet.name, sheet_index: sheet.index,
      completed: (pass - 1) * sheetCount + completedSheets,
      total: passes * sheetCount, pass, passes, updated_unix_ms: Date.now()
    };
  }

  function createWorker(helpers, transport, expectedWorkbook) {
    const {office, bounds, checkedWorksheet, rowHeights, hideGridlines, verifyDrawingBounds} = helpers;
    const captures = new Map();
    const geometries = new Map();
    // Retained PNGs are immutable evidence. Later upload commands send these
    // exact bytes, even if Excel's current rendering has changed meanwhile.
    const retainedPngs = new Map();
    let workbookVerified = false;
    let pendingPngCharacters = 0, activeBatchPixels = 0, activeBatchTiles = 0;
    let pendingEncodedBytes = 0;
    let peakPendingEncodedBytes = 0;
    let pipelineOverlapCount = 0;
    let timings;
    let lastCommandTiming;
    let captureProgress = null;
    let sweepProgress = null;

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
        retained_image_tiles: retainedPngs.size,
        retained_image_bytes: retainedImageBytes(),
        retained_tiles: [...captures.values()].reduce((sum, plan) => sum + plan.size, 0),
        geometry_cache_entries: geometries.size,
        geometry_rows: [...geometries.values()].reduce((sum, geometry) => sum + geometry.rows.length, 0),
        pending_png_characters: pendingPngCharacters,
        pending_encoded_bytes: pendingEncodedBytes,
        peak_pending_encoded_bytes: peakPendingEncodedBytes,
        pipeline_overlap_count: pipelineOverlapCount,
        active_batch_pixels: activeBatchPixels, active_batch_tiles: activeBatchTiles,
        script_evaluations: root.__excelShrinkPaneRuntime?.evaluations ?? 0
      };
      if (lastCommandTiming) sample.last_command_timing = {...lastCommandTiming};
      if (captureProgress) sample.capture_progress = {...captureProgress};
      if (sweepProgress) sample.sweep_progress = {...sweepProgress};
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
        const heights = await rowHeights(state.sheet);
        if (heights.runs.length !== state.height_runs.length || heights.runs.some((run, index) =>
          run.start_row !== state.height_runs[index].start_row ||
          run.end_row !== state.height_runs[index].end_row ||
          run.height !== state.height_runs[index].height)) {
          throw new Error(`Row heights changed after planning ${state.sheet.name}`);
        }
        await geometry.read(state.end, heights.runs);
        await geometry.calibrate();
        await verifyDrawingBounds(state.sheet, geometry, state.end);
      }
      rememberGeometry(state.geometry_key, geometry);
      return geometry;
    }

    function reserveImageBytes(bytes) {
      pendingEncodedBytes += bytes;
      peakPendingEncodedBytes = Math.max(peakPendingEncodedBytes, pendingEncodedBytes);
    }

    async function finishImage(state, tile, base64, {hash = true, accountMemory = false} = {}) {
      const {sheet, end} = state;
      const geometry = await geometryFor(state);
      const dimensions = CaptureGeometry.pngSize(base64);
      if (dimensions.width * dimensions.height > (tile.pixelBudget ?? geometry.maxPixels)) {
        const error = new Error('Image exceeds the tile pixel budget');
        error.code = 'IMAGE_BUDGET';
        throw error;
      }
      if (dimensions.width !== tile.width || dimensions.height !== tile.expectedHeight) {
        throw new Error(`Image dimensions changed on ${sheet.name}`);
      }
      const final = await CaptureOverflow.crop(base64, 0, tile.cropY, geometry.imageWidth(end), tile.cropHeight);
      const size = CaptureGeometry.pngSize(final);
      if (size.width !== geometry.imageWidth(end) || size.height !== tile.cropHeight) {
        throw new Error('Image crop changed dimensions');
      }
      // Track the transient strings and decoded buffer for observability.
      // The image geometry and transport validate each capture independently.
      const decodedBytes = Math.ceil(final.length / 4) * 3;
      // SubtleCrypto may also copy its input while digesting.
      const temporaryBytes = final.length * 2 + decodedBytes * (hash ? 4 : 3);
      if (accountMemory) {
        reserveImageBytes(temporaryBytes);
      }
      let image;
      try {
        image = {...await encoded(final, hash), ...size};
        if (timings) timings.hash_ms += image.hash_ms ?? 0;
      } finally {
        if (accountMemory) {
          pendingEncodedBytes -= temporaryBytes;
        }
      }
      if (accountMemory) {
        reserveImageBytes(image.bytes.byteLength);
      }
      return image;
    }

    async function render(state, tile, hash = true, accountMemory = false) {
      await geometryFor(state);
      const base64 = await measure('capture_ms', () => office(async context => {
        const image = context.workbook.worksheets.getItem(state.sheet.name)
          .getRange(CaptureGeometry.address(tile.raw)).getImage();
        await context.sync();
        return image.value;
      }));
      return measure('finish_image_ms', () => finishImage(state, tile, base64, {hash, accountMemory}));
    }

    /** Batch Office reads, then hash/release each image before requesting more. */
    async function* sweepImages(state, tiles, limit, planOnly) {
      const geometry = await geometryFor(state);
      for (let first = 0; first < tiles.length;) {
        let end = first, pixels = 0;
        while (end < tiles.length && end - first < limit) {
          const next = tiles[end].width * tiles[end].expectedHeight;
          if (end > first && pixels + next > 64000000) break;
          pixels += next;
          end++;
        }
        const chunk = tiles.slice(first, end);
        activeBatchTiles = chunk.length;
        activeBatchPixels = pixels;
        let base64s = [];
        let remainingCharacters = 0;
        try {
          if (!planOnly) {
            base64s = await measure('capture_ms', () => office(async context => {
              const worksheet = context.workbook.worksheets.getItem(state.sheet.name);
              const images = chunk.map(tile => worksheet.getRange(CaptureGeometry.address(tile.raw)).getImage());
              await context.sync();
              return images.map(image => image.value);
            }));
            timings.capture_syncs++;
            timings.max_capture_batch_tiles = Math.max(timings.max_capture_batch_tiles, chunk.length);
            remainingCharacters = base64s.reduce((sum, value) => sum + value.length, 0);
            reserveImageBytes(remainingCharacters * 2);
            pendingPngCharacters += remainingCharacters;
          }
          for (let local = 0; local < chunk.length; local++) {
            const tile = chunk[local];
            const image = planOnly
              ? {width: geometry.imageWidth(state.end), height: tile.cropHeight, sha256: '0'.repeat(64)}
              : await measure('finish_image_ms', () => finishImage(state, tile, base64s[local], {accountMemory: true}));
            const characters = base64s[local]?.length ?? 0;
            base64s[local] = null;
            remainingCharacters -= characters;
            pendingPngCharacters -= characters;
            pendingEncodedBytes -= characters * 2;
            try {
              yield {index: first + local, image};
            } finally {
              if (image.bytes) {
                pendingEncodedBytes -= image.bytes.byteLength;
                image.bytes = null;
              }
            }
          }
        } finally {
          base64s.fill(null);
          pendingPngCharacters -= remainingCharacters;
          pendingEncodedBytes -= remainingCharacters * 2;
          activeBatchTiles = 0;
          activeBatchPixels = 0;
        }
        first = end;
      }
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
            command.tile_megapixels < 1 || command.tile_megapixels > 64))) {
        throw new Error('Incomplete resident capture command');
      }
      await verifyWorkbook();
      const result = {heights: [], sheets: [], tiles: [], planning_timings: [], sheet_timings: []};
      if (captures.has(command.id)) throw new Error('Capture command was already executed');
      const retained = new Map();
      for (const sheet of command.sheets) {
        const sheetStartedAt = performance.now();
        const captureBefore = timings?.capture_ms ?? 0;
        const finishBefore = timings?.finish_image_ms ?? 0;
        const hashBefore = timings?.hash_ms ?? 0;
        if (command.kind === 'capture') {
          sweepProgress = sheetSweepProgress(command, result.sheets.length, sheet);
          captureProgress = {command_id: command.id, kind: command.kind,
            sheet_index: sheet.index, sheet_name: sheet.name, completed: 0, total: null,
            updated_unix_ms: Date.now()};
        }
        helpers.phase(command.kind === 'plan' ? 'Planning capture tiles' : 'Capturing tile checksums');
        helpers.progress(result.sheets.length, command.sheets.length, sheet.name);
        const {bottom, right} = bounds(sheet, true);
        await checkedWorksheet(sheet);
        const measuredAt = performance.now();
        const heights = await rowHeights(sheet);
        result.heights.push(heights);
        const rowHeightMs = performance.now() - measuredAt;
        await hideGridlines(sheet);
        const geometry = new CaptureGeometry.SheetGeometry(office, sheet.name, bottom, right,
          command.tile_megapixels ?? 32);
        const marginPixels = sheet.right_margin_pixels ?? 1920;
        const maximum = sheet.capture_right_column ?? 16383;
        const negotiated = sheet.capture_end_column;
        if (!Number.isInteger(maximum) || maximum < right || maximum >= 16384) {
          throw new Error('Invalid maximum capture boundary');
        }
        // Narrow or hidden columns extend the measured window on demand.
        const readEnd = Math.min(maximum, Math.max(right + 32, negotiated ?? right));
        const geometryAt = performance.now();
        await geometry.read(readEnd, heights.runs);
        const geometryMs = performance.now() - geometryAt;
        const calibrationAt = performance.now();
        await geometry.calibrate();
        const proposed = await geometry.rightMarginEnd(right, marginPixels);
        const calibrationMs = performance.now() - calibrationAt;
        const end = negotiated ?? proposed;
        if (!Number.isInteger(end) || end < proposed || end > maximum) {
          throw new Error('Negotiated capture boundary is outside the fixed-margin extent');
        }
        const actualMargin = geometry.imageWidth(end) - geometry.imageWidth(right);
        const tilesAt = performance.now();
        const tiles = selectedTiles(geometry, end, sheet.tile_rows, {planOnly: command.kind === 'plan'});
        result.planning_timings.push({sheet_index: sheet.index, sheet_name: sheet.name,
          row_height_ms: rowHeightMs, geometry_ms: geometryMs,
          calibration_ms: calibrationMs, tile_split_ms: performance.now() - tilesAt});
        if (captureProgress) captureProgress.total = tiles.length;
        // Bound aggregate metadata before retaining another sheet. Per-image
        // and selected-capture limits apply separately when images are needed.
        if (result.tiles.length + tiles.length > MAX_PLANNED_TILES_PER_COMMAND) {
          const error = new Error('Workbook exceeds the planned tile metadata budget');
          error.code = 'IMAGE_BUDGET';
          throw error;
        }
        const drawingBounds = await verifyDrawingBounds(sheet, geometry, end);
        const state = {sheet, right, end, height_runs: heights.runs,
          geometry_key: `${command.id}:${sheet.index}`,
          megapixels: command.tile_megapixels ?? 32};
        rememberGeometry(state.geometry_key, geometry);
        result.sheets.push({sheet_index: sheet.index, sheet_name: sheet.name,
          drawing_bounds: drawingBounds,
          right_margin: {requested_pixels: marginPixels, actual_pixels: actualMargin,
            limited_by_grid: end === 16383 && actualMargin < marginPixels},
          capture_right_column: end, tile_rows: tiles.map(tile => [tile.core.r0, tile.core.r1])});
        const batchTiles = command.batch_tiles ?? 1;
        if (!Number.isInteger(batchTiles) || batchTiles < 1 || batchTiles > 1024) {
          throw new Error('Sweep batch must contain 1 to 1024 captures');
        }
        for await (const {index, image} of sweepImages(state, tiles, batchTiles, command.kind === 'plan')) {
          const tile = tiles[index];
          const metadata = {sheet_index: sheet.index, sheet_name: sheet.name, tile_index: index,
            range: CaptureGeometry.address(tile.core), core_rows: [tile.core.r0, tile.core.r1],
            x: 0, y: geometry.pixelY[tile.core.r0],
            capture_pixels: tile.width * tile.expectedHeight,
            pixel_budget: tile.pixelBudget ?? geometry.maxPixels,
            capture_budget_fallback: tile.budgetFallback ?? null,
            capture_right_column: end, width: image.width, height: image.height, sha256: image.sha256};
          result.tiles.push(metadata);
          if (captureProgress) {
            captureProgress.completed = index + 1;
            captureProgress.updated_unix_ms = Date.now();
          }
          retained.set(key(metadata), {...state, tile, metadata});
          // sweepImages releases image bytes as soon as this iteration finishes.
          if (command.kind !== 'plan') helpers.phase(`Hashing ${sheet.name}: tile ${index + 1}/${tiles.length}`);
          // No encoded or decoded images are retained while waiting for peers.
        }
        if (command.kind === 'capture') {
          sweepProgress = sheetSweepProgress(command, result.sheets.length, sheet);
        }
        result.sheet_timings.push({...result.planning_timings.at(-1),
          selected_tiles: tiles.length, elapsed_ms: performance.now() - sheetStartedAt,
          capture_ms: (timings?.capture_ms ?? 0) - captureBefore,
          finish_image_ms: (timings?.finish_image_ms ?? 0) - finishBefore,
          hash_ms: (timings?.hash_ms ?? 0) - hashBefore});
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
        captureProgress = {command_id: command.id, kind: command.kind,
          sheet_index: state.sheet.index, sheet_name: state.sheet.name, completed: 0, total: 1,
          updated_unix_ms: Date.now()};
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
        captureProgress.completed = 1;
        captureProgress.updated_unix_ms = Date.now();
      }
      return result;
    }

    const retainedKey = (captureId, tile) => `${captureId}:${key(tile)}`;

    function retainedImageBytes() {
      let bytes = 0;
      for (const entry of retainedPngs.values()) bytes += entry.image.bytes.byteLength;
      return bytes;
    }

    function discardImage(id) {
      const retained = retainedPngs.get(id);
      if (!retained) return;
      pendingEncodedBytes -= retained.image.bytes.byteLength;
      retained.image.bytes = null;
      retainedPngs.delete(id);
    }

    function checkRetainedBudget(states) {
      let pixels = states.reduce((sum, state) => sum + state.metadata.capture_pixels, 0);
      for (const entry of retainedPngs.values()) pixels += entry.metadata.capture_pixels;
      if (retainedPngs.size + states.length > 16 || pixels > 64000000) {
        const error = new Error('Retained images and new captures exceed the 16 tile / 64 megapixel budget');
        error.code = 'IMAGE_BUDGET';
        throw error;
      }
    }

    function requestedRetained(command) {
      if (!captures.has(command.capture_id) || !Array.isArray(command.tiles) ||
          !command.tiles.length || command.tiles.length > 16) {
        throw new Error('Invalid retained-image request');
      }
      const seen = new Set();
      return command.tiles.map(tile => {
        const id = retainedKey(command.capture_id, tile);
        const entry = retainedPngs.get(id);
        if (!entry || seen.has(id) || ['sheet_name', 'range', 'width', 'height', 'sha256',
          'x', 'y', 'capture_pixels', 'capture_right_column'].some(field => tile[field] !== entry.metadata[field]) ||
          !Array.isArray(tile.core_rows) || tile.core_rows.length !== 2 ||
          tile.core_rows.some((row, index) => row !== entry.metadata.core_rows[index])) {
          throw new Error('Retained image no longer matches the requested capture');
        }
        seen.add(id);
        return entry;
      });
    }

    async function uploadRetained(command) {
      const images = requestedRetained(command);
      const result = {images: []};
      helpers.phase('Uploading retained capture images');
      for (const {metadata, image} of images) {
        const receipt = await measure('upload_ms', () => transport.image(command.id, metadata, image.bytes));
        if (typeof receipt.image_path !== 'string' || !receipt.image_path ||
            receipt.sha256 !== metadata.sha256 || receipt.width !== metadata.width ||
            receipt.height !== metadata.height) {
          throw new Error('Missing or invalid retained-image receipt');
        }
        result.images.push({...metadata, ...receipt});
      }
      return result;
    }

    function discardRetained(command) {
      if (typeof command.capture_id !== 'string' || !command.capture_id ||
          !Array.isArray(command.tiles) || command.tiles.length > 16 ||
          command.tiles.some(tile => !Number.isInteger(tile.sheet_index) ||
            !Number.isInteger(tile.tile_index) || tile.sheet_index < 0 || tile.tile_index < 0)) {
        throw new Error('Invalid retained-image release');
      }
      for (const tile of command.tiles) discardImage(retainedKey(command.capture_id, tile));
      return {};
    }

    // Both transport modes capture identical finalized PNGs. Live mode uploads
    // immediately; hash-first mode retains them until the coordinator decides
    // whether they are needed as evidence or neighboring context.
    async function captureImages(command) {
      const hashFirst = command.kind === 'capture_hashes';
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
      const boundaries = command.capture_batches === undefined
        ? [states.length]
        : command.capture_batches;
      if (!Array.isArray(boundaries) || boundaries.length < 1 || boundaries.length > 2 ||
          boundaries.at(-1) !== states.length || boundaries.some((end, index) =>
            !Number.isInteger(end) || end <= (boundaries[index - 1] ?? 0))) {
        throw new Error('Capture batch boundaries must cover the tiles in one or two ordered chunks');
      }
      checkRetainedBudget(states);
      if (hashFirst && states.some(state => retainedPngs.has(retainedKey(command.capture_id, state.metadata)))) {
        throw new Error('Capture image is already retained; release it before capturing again');
      }
      helpers.phase(`Capturing synchronized batch (${states.length} tiles)`);
      captureProgress = {command_id: command.id, kind: command.kind,
        sheet_index: states[0].sheet.index, sheet_name: states[0].sheet.name,
        completed: 0, total: states.length, updated_unix_ms: Date.now()};
      await checkedWorksheet(states[0].sheet);
      await geometryFor(states[0]);
      activeBatchTiles = states.length;
      activeBatchPixels = states.reduce((sum, state) => sum + state.tile.width * state.tile.expectedHeight, 0);
      const result = hashFirst ? {tiles: []} : {images: []};
      const retainedImages = new Set();
      const addedRetained = [];
      let completed = false;
      let uploadTask = Promise.resolve();
      let uploadError;
      let uploadPending = false;
      let stopping = false;

      function checkUpload() {
        if (uploadError) {
          throw uploadError;
        }
      }

      function releaseImage(image) {
        if (!retainedImages.delete(image)) {
          return;
        }
        pendingEncodedBytes -= image.bytes.byteLength;
        image.bytes = null;
      }

      async function uploadImage(state, image) {
        try {
          const receipt = await measure('upload_ms', () => transport.image(command.id,
            {...state.metadata, sha256: undefined}, image.bytes));
          if (typeof receipt.image_path !== 'string' || !receipt.image_path ||
              typeof receipt.sha256 !== 'string' || receipt.width !== image.width || receipt.height !== image.height) {
            throw new Error('Missing or invalid image-first receipt');
          }
          result.images.push({...state.metadata, ...receipt});
        } finally {
          releaseImage(image);
        }
      }

      async function uploadChunk(chunk, images) {
        for (let index = 0; index < images.length && !stopping; index++) {
          await uploadImage(chunk[index], images[index]);
        }
      }

      try {
        let first = 0;
        for (const end of boundaries) {
          checkUpload();
          const chunk = states.slice(first, end);
          if (uploadPending) {
            pipelineOverlapCount++;
          }
          const base64s = await measure('capture_ms', () => office(async context => {
            const worksheet = context.workbook.worksheets.getItem(chunk[0].sheet.name);
            const images = chunk.map(state =>
              worksheet.getRange(CaptureGeometry.address(state.tile.raw)).getImage());
            await context.sync();
            return images.map(image => image.value);
          }));
          const rawCharacters = base64s.reduce((sum, value) => sum + value.length, 0);
          let remainingCharacters = 0;
          const finalized = [];
          try {
            // Office returns complete strings, so their encoded size is only
            // known after sync. Check before retaining/decoding; the pixel and
            // chunk limits bound the capture allocation itself.
            reserveImageBytes(rawCharacters * 2);
            remainingCharacters = rawCharacters;
            pendingPngCharacters += rawCharacters;
            for (let index = 0; index < chunk.length; index++) {
              checkUpload();
              const state = chunk[index];
              const image = await measure('finish_image_ms', () => finishImage(
                state, state.tile, base64s[index], {hash: hashFirst, accountMemory: true}
              ));
              retainedImages.add(image);
              const characters = base64s[index].length;
              remainingCharacters -= characters;
              pendingPngCharacters -= characters;
              pendingEncodedBytes -= characters * 2;
              base64s[index] = null;
              checkUpload();
              if (hashFirst) {
                const metadata = {...state.metadata, sha256: image.sha256, png_bytes: image.bytes.byteLength};
                const id = retainedKey(command.capture_id, metadata);
                retainedPngs.set(id, {captureId: command.capture_id, metadata, image});
                addedRetained.push(id);
                retainedImages.delete(image);
                result.tiles.push(metadata);
              } else if (command.capture_batches === undefined) {
                // Preserve the reference path: capture once, then finalize and
                // upload each tile before finalizing the next.
                await uploadImage(state, image);
              } else {
                finalized.push(image);
              }
              captureProgress.completed++;
              captureProgress.updated_unix_ms = Date.now();
            }
            await uploadTask;
            checkUpload();
            if (finalized.length) {
              // Finish every Office operation
              // before starting the next chunk. Only uploads may overlap it.
              uploadPending = true;
              uploadTask = uploadChunk(chunk, finalized)
                .catch(error => {
                  uploadError = error;
                })
                .finally(() => {
                  uploadPending = false;
                });
            }
          } finally {
            base64s.fill(null);
            pendingPngCharacters -= remainingCharacters;
            pendingEncodedBytes -= remainingCharacters * 2;
          }
          first = end;
        }
        await uploadTask;
        checkUpload();
        completed = true;
        return result;
      } finally {
        stopping = true;
        // The HTTP transport has a request deadline. Do not return a failed
        // command while one of its uploads can still mutate host state.
        await uploadTask;
        for (const image of retainedImages) {
          releaseImage(image);
        }
        if (!completed) {
          for (const id of addedRetained) discardImage(id);
        }
      }
    }

    return {
      verifyWorkbook,
      resources,
      recordAcknowledgment(command, measured, acknowledgment) {
        lastCommandTiming = {command_id: command.id, kind: command.kind, ...measured, ...acknowledgment};
      },
      async execute(command) {
        const started = performance.now();
        peakPendingEncodedBytes = pendingEncodedBytes;
        pipelineOverlapCount = 0;
        timings = {capture_ms: 0, finish_image_ms: 0, hash_ms: 0, upload_ms: 0, capture_syncs: 0, max_capture_batch_tiles: 0};
        let result;
        try {
          if (command.kind === 'capture' || command.kind === 'plan') result = await capture(command);
          else if (command.kind === 'images') result = await images(command);
          else if (command.kind === 'capture_images' || command.kind === 'capture_hashes') result = await captureImages(command);
          else if (command.kind === 'diagnosis') {
            if (!root.ResidentDiagnosis?.run) throw new Error('Resident diagnosis helper is unavailable');
            result = await root.ResidentDiagnosis.run(command, {office, bounds, checkedWorksheet, rowHeights, helpers, transport});
            if (command.operation !== 'read_cells') geometries.clear();
          }
          else if (command.kind === 'upload_retained') result = await uploadRetained(command);
          else if (command.kind === 'discard_retained') result = discardRetained(command);
          else if (command.kind === 'release') {
            for (const [id, entry] of retainedPngs) {
              if (!command.capture_id || entry.captureId === command.capture_id) discardImage(id);
            }
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
          result.timings = {
            ...timings,
            execution_ms: result.elapsed_ms,
            peak_pending_encoded_bytes: peakPendingEncodedBytes,
            pipeline_overlap_count: pipelineOverlapCount
          };
          lastCommandTiming = {command_id: command.id, kind: command.kind, ...result.timings};
          result.checksum_tiles = ['capture', 'capture_hashes'].includes(command.kind) ? result.tiles?.length ?? 0 : 0;
          result.uploaded_images = result.images?.length ?? 0;
          return result;
        } finally {
          captureProgress = null;
          pendingPngCharacters = 0;
          pendingEncodedBytes = retainedImageBytes();
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
    }, diagnosisImage(id, bytes) {
      return request(`/diagnosis-image/${clientPath}/${encodeURIComponent(id)}`,
        bytes, {'Content-Type': 'application/octet-stream'});
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

  const api = {run, createWorker, encoded, selectedTiles, sheetSweepProgress};
  root.ResidentCapture = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
