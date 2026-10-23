(function (root) {
  'use strict';

  let config, request, helpers;
  const prepared = new Map();
  const metrics = {
    office_ms: 0, capture_ms: 0, decode_ms: 0, hash_ms: 0,
    round_wait_ms: 0, refine_wait_ms: 0, refine_ms: 0, upload_wait_ms: 0,
    png_bytes: 0, image_bytes: 0, batches: 0, max_batch: 0,
    first_round_tiles: 0, refinement_tiles: 0, evidence_images: 0
  };
  function record(name, value) { metrics[name] = (metrics[name] || 0) + value; }
  function begin() { for (const name of Object.keys(metrics)) metrics[name] = 0; }
  function batch(count) {
    record('batches', 1);
    metrics.max_batch = Math.max(metrics.max_batch, count);
  }
  function batchEnd(tiles, offset, count, budget) {
    let end = offset, pixels = 0;
    while (end < tiles.length && end - offset < count) {
      const size = tiles[end].width * tiles[end].expectedHeight;
      if (size > budget) throw new Error('One tile exceeds the batch pixel budget');
      if (pixels + size > budget) break;
      pixels += size;
      end++;
    }
    return end;
  }
  function configure(value, send, functions) {
    config = value;
    request = send;
    helpers = functions;
  }
  async function bytes(base64) {
    const before = performance.now();
    const binary = atob(base64), png = new Uint8Array(binary.length);
    for (let start = 0; start < binary.length; start += 1048576) {
      const end = Math.min(start + 1048576, binary.length);
      for (let i = start; i < end; i++) png[i] = binary.charCodeAt(i);
      if (start) await new Promise(resolve => setTimeout(resolve, 0));
    }
    record('decode_ms', performance.now() - before);
    record('png_bytes', png.length);
    return png;
  }
  async function encoded(base64, keepBytes) {
    const size = CaptureGeometry.pngSize(base64);
    const png = await bytes(base64);
    const before = performance.now();
    const sha256 = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', png)),
      byte => byte.toString(16).padStart(2, '0')).join('');
    record('hash_ms', performance.now() - before);
    return {width: size.width, height: size.height, sha256, png: keepBytes ? png : null};
  }
  async function prepareSheet(sheet) {
    const {bottom, right} = helpers.bounds(sheet, true);
    const targets = helpers.targetRows(sheet, bottom);
    await helpers.checkedWorksheet(sheet);
    await helpers.hideGridlines(sheet);
    const geometry = new CaptureGeometry.SheetGeometry(helpers.office, sheet.name, bottom, right,
      config.tile_megapixels ?? 32);
    const probe = sheet.horizontal_overflow_required && right < 16383;
    const sharedEnd = sheet.capture_right_column;
    if (sharedEnd !== undefined && (!Number.isInteger(sharedEnd) || sharedEnd < right ||
        sharedEnd >= 16384 || (probe && sharedEnd === right))) {
      throw new Error(`Invalid shared capture boundary for ${sheet.name}`);
    }
    const maxColumn = sharedEnd !== undefined ? sharedEnd : probe ? Math.min(16383, right + 256) : right;
    await geometry.read(maxColumn);
    await geometry.calibrate();
    if (probe && geometry.fixedWidth !== null) {
      throw new Error(`Cannot check right-edge overflow on fixed-width sheet ${sheet.name}`);
    }
    const proposedEnd = probe ? CaptureGeometry.probeEnd(geometry, right) : right;
    let end = sharedEnd !== undefined ? sharedEnd : proposedEnd;
    let tiles = geometry.tiles(end, targets, sheet.tile_rows);
    const shared = await request('/pair/geometry', {
      run_id: config.run_id, side: config.side, sheet_index: sheet.index,
      end_column: proposedEnd, max_end_column: end,
      tile_rows: tiles.map(tile => [tile.core.r0, tile.core.r1])
    });
    if (!Number.isInteger(shared.capture_right_column) ||
        shared.capture_right_column < proposedEnd || shared.capture_right_column > end) {
      throw new Error(`Invalid negotiated capture boundary for ${sheet.name}`);
    }
    end = shared.capture_right_column;
    tiles = geometry.tiles(end, targets, shared.tile_rows);
    await helpers.verifyDrawingBounds(sheet, geometry, end);
    prepared.set(sheet.index, {sheet, right, end, probe, geometry, tiles});
  }
  async function prepareAll(sheets) {
    for (const sheet of sheets) await prepareSheet(sheet);
  }
  async function captureBatch(sheet, tiles, offset, end) {
    const before = performance.now();
    const images = await helpers.office(async context => {
      const worksheet = context.workbook.worksheets.getItem(sheet.name);
      const pending = tiles.slice(offset, end).map(tile =>
        worksheet.getRange(CaptureGeometry.address(tile.raw)).getImage());
      await context.sync();
      return pending.map(image => image.value);
    });
    record('capture_ms', performance.now() - before);
    batch(images.length);
    return images;
  }
  async function firstRoundTile(state, tile, index, base64) {
    const {sheet, right, end, probe, geometry} = state;
    const size = CaptureGeometry.pngSize(base64);
    if (size.width !== tile.width || size.height !== tile.expectedHeight) {
      throw new Error(`Image dimensions changed at ${sheet.name}!${CaptureGeometry.address(tile.raw)}`);
    }
    const overflow = probe
      ? await CaptureOverflow.check(helpers.office, sheet.name, geometry, tile, right, base64)
      : {rightEdge: false};
    if (overflow.rightEdge) {
      throw new Error(`Text reaches the unverified right edge at ${sheet.name}!${CaptureGeometry.address(tile.raw)}`);
    }
    const width = geometry.imageWidth(end);
    const finalPng = await CaptureOverflow.crop(base64, 0, tile.cropY, width, tile.cropHeight);
    const finalSize = CaptureGeometry.pngSize(finalPng);
    if (finalSize.width !== width || finalSize.height !== tile.cropHeight) {
      throw new Error('Image crop changed dimensions');
    }
    const content = await encoded(finalPng, false);
    record('first_round_tiles', 1);
    return {tile_index: index, range: CaptureGeometry.address(tile.core),
      width: content.width, height: content.height, sha256: content.sha256};
  }
  function checkedDivergent(reply, expected, name) {
    if (!reply || !Array.isArray(reply.divergent) ||
        reply.divergent.some(id => !expected.includes(id)) ||
        new Set(reply.divergent).size !== reply.divergent.length) {
      throw new Error(`Invalid ${name} divergence response`);
    }
    return expected.filter(id => reply.divergent.includes(id));
  }
  function split(rect, parentId) {
    const rows = rect.r1 - rect.r0, columns = rect.c1 - rect.c0;
    if (rows === 1 && columns === 1) return [];
    const r = rows > 1 ? Math.floor((rect.r0 + rect.r1) / 2) : rect.r1;
    const c = columns > 1 ? Math.floor((rect.c0 + rect.c1) / 2) : rect.c1;
    const rowParts = rows > 1 ? [[rect.r0, r], [r, rect.r1]] : [[rect.r0, rect.r1]];
    const columnParts = columns > 1 ? [[rect.c0, c], [c, rect.c1]] : [[rect.c0, rect.c1]];
    const children = [];
    for (const [r0, r1] of rowParts) for (const [c0, c1] of columnParts) {
      children.push({id: `${parentId}.${children.length}`, rect: {r0, r1, c0, c1}});
    }
    return children;
  }
  async function captureParts(sheet, parts) {
    const before = performance.now();
    const images = await helpers.office(async context => {
      const worksheet = context.workbook.worksheets.getItem(sheet.name);
      const pending = parts.map(part => worksheet.getRange(CaptureGeometry.address(part.rect)).getImage());
      await context.sync();
      return pending.map(image => image.value);
    });
    record('capture_ms', performance.now() - before);
    batch(images.length);
    return await Promise.all(images.map(async (image, index) => {
      const content = await encoded(image, true);
      record('refinement_tiles', 1);
      return {...parts[index], ...content, range: CaptureGeometry.address(parts[index].rect)};
    }));
  }
  async function evidence(sheetIndex, part) {
    const before = performance.now();
    const response = await fetch('/evidence/' + encodeURIComponent(config.run_id) + '/' +
      sheetIndex + '/' + encodeURIComponent(part.id), {
      method: 'POST', headers: {'Content-Type': 'image/png', 'X-Sha256': part.sha256},
      body: part.png
    });
    if (!response.ok) throw new Error('Evidence upload: ' + response.status + ' ' + await response.text());
    await response.json();
    record('upload_wait_ms', performance.now() - before);
    record('image_bytes', part.png.length);
    record('evidence_images', 1);
  }
  async function refine(sheet, node) {
    const children = split(node.rect, node.id);
    if (!children.length) {
      // A root tile may be a single cell. The initial tile includes a halo and
      // a crop, so a direct cell capture needs its own hash offer.
      const leaf = (await captureParts(sheet, [{id: `${node.id}.0`, rect: node.rect}]))[0];
      const before = performance.now();
      const reply = await request('/refine', {
        run_id: config.run_id, sheet_index: sheet.index, parent_id: node.id,
        parts: [{id: leaf.id, range: leaf.range, width: leaf.width,
          height: leaf.height, sha256: leaf.sha256}]
      });
      record('refine_wait_ms', performance.now() - before);
      if (checkedDivergent(reply, [leaf.id], 'singleton refinement').length) {
        await evidence(sheet.index, leaf);
      }
      return;
    }
    const captured = await captureParts(sheet, children);
    const before = performance.now();
    const reply = await request('/refine', {
      run_id: config.run_id, sheet_index: sheet.index, parent_id: node.id,
      parts: captured.map(({id, range, width, height, sha256}) => ({id, range, width, height, sha256}))
    });
    record('refine_wait_ms', performance.now() - before);
    const divergent = checkedDivergent(reply, captured.map(part => part.id), 'refinement');
    for (const part of captured) if (!divergent.includes(part.id)) part.png = null;
    for (const part of captured) {
      if (!divergent.includes(part.id)) continue;
      if (split(part.rect, part.id).length) {
        part.png = null;
        await refine(sheet, part);
      } else {
        await evidence(sheet.index, part);
        part.png = null;
      }
    }
  }
  async function captureSheet(sheet) {
    const state = prepared.get(sheet.index);
    if (!state || state.sheet.name !== sheet.name) throw new Error('Sheet was not prepared at the start line');
    const {tiles} = state;
    const offers = [];
    for (let offset = 0; offset < tiles.length;) {
      const end = batchEnd(tiles, offset, config.batch_size, config.max_batch_pixels);
      const images = await captureBatch(sheet, tiles, offset, end);
      for (let index = offset; index < end; index++) {
        offers.push(await firstRoundTile(state, tiles[index], index, images[index - offset]));
      }
      offset = end;
    }
    const before = performance.now();
    const reply = await request('/round', {run_id: config.run_id, sheet_index: sheet.index, tiles: offers});
    record('round_wait_ms', performance.now() - before);
    const divergent = checkedDivergent(reply, offers.map(tile => tile.tile_index), 'first round');
    const refinementBegan = performance.now();
    for (const index of divergent) {
      const tile = tiles[index];
      await refine(sheet, {id: `t${index}`, rect: tile.core, sha256: offers[index].sha256});
    }
    record('refine_ms', performance.now() - refinementBegan);
    return [];
  }
  const api = {metrics, record, batchEnd, configure, prepareAll, captureSheet, begin, split};
  root.Experiment = api;
  if (typeof module !== 'undefined') module.exports = api;
}(typeof window === 'undefined' ? globalThis : window));
