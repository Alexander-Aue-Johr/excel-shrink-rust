(function (root) {
  'use strict';

  function yieldToHost() {
    return new Promise(resolve => {
      const channel = new MessageChannel();
      channel.port1.onmessage = () => {
        channel.port1.close();
        channel.port2.close();
        resolve();
      };
      channel.port2.postMessage(null);
    });
  }

  async function loadImage(base64) {
    const image = new Image();
    await new Promise((resolve, reject) => {
      image.onload = () => { image.onload = null; image.onerror = null; resolve(); };
      image.onerror = () => {
        image.onload = null; image.onerror = null; image.src = '';
        reject(new Error('Cannot decode Excel PNG'));
      };
      image.src = `data:image/png;base64,${base64}`;
    });
    return image;
  }

  async function decode(base64, pixels = true) {
    const image = await loadImage(base64);
    const canvas = document.createElement('canvas');
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const context = canvas.getContext('2d', {willReadFrequently: true});
    if (!context) throw new Error('Canvas pixels are unavailable');
    context.drawImage(image, 0, 0);
    image.src = '';
    const width = canvas.width, height = canvas.height;
    const data = pixels ? context.getImageData(0, 0, width, height).data : null;
    if (pixels) { canvas.width = 0; canvas.height = 0; }
    return {canvas, width, height, pixels: data};
  }

  function release(image) {
    if (image) { image.pixels = null; image.canvas.width = 0; image.canvas.height = 0; }
  }

  async function sameImage(left, right) {
    if (left.width !== right.width || left.height !== right.height) return false;
    for (let start = 0; start < left.pixels.length; start += 1048576) {
      if (start) await yieldToHost();
      const end = Math.min(start + 1048576, left.pixels.length);
      for (let index = start; index < end; index++) {
        if (left.pixels[index] !== right.pixels[index]) return false;
      }
    }
    return true;
  }

  function sheetRow(geometry, tile, pixelY) {
    const position = geometry.pixelY[tile.raw.r0] + pixelY;
    let low = tile.raw.r0, high = tile.raw.r1;
    while (low + 1 < high) {
      const middle = Math.floor((low + high) / 2);
      if (geometry.pixelY[middle] <= position) low = middle;
      else high = middle;
    }
    return low;
  }

  async function candidateRows(image, geometry, tile, baseEdge) {
    const rows = new Set();
    const width = image.width;
    if (baseEdge >= width - 2) return [];
    const firstY = tile.cropY + 1;
    const lastY = Math.min(image.height - 1, tile.cropY + tile.cropHeight - 1);
    const rowsPerTurn = Math.max(1, Math.floor(262144 / width));
    for (let y = firstY; y < lastY; y++) {
      if (y > firstY && (y - firstY) % rowsPerTurn === 0) await yieldToHost();
      const background = (y * width + width - 2) * 4;
      let signal = false;
      for (let x = baseEdge; x < width - 2; x++) {
        const offset = (y * width + x) * 4;
        if (image.pixels[offset] !== image.pixels[background] ||
            image.pixels[offset + 1] !== image.pixels[background + 1] ||
            image.pixels[offset + 2] !== image.pixels[background + 2] ||
            image.pixels[offset + 3] !== image.pixels[background + 3]) {
          signal = true;
          break;
        }
      }
      if (signal) {
        const row = sheetRow(geometry, tile, y);
        if (row >= tile.core.r0 && row < tile.core.r1) rows.add(row);
      }
      if (rows.size > 2048) throw new Error('Too many right-edge candidate rows for a bounded check');
    }
    return [...rows].sort((a, b) => a - b);
  }

  async function safeTargets(office, sheet, rows, column) {
    const targets = [];
    for (let offset = 0; offset < rows.length; offset += 128) {
      const batch = rows.slice(offset, offset + 128);
      const items = await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet);
        worksheet.protection.load('protected');
        const cells = batch.map(row => {
          const cell = worksheet.getRangeByIndexes(row, column, 1, 1);
          cell.load('values,formulas,valueTypes,hasSpill');
          cell.format.load('rowHeight');
          const merges = cell.getMergedAreasOrNullObject();
          merges.load('address');
          const near = worksheet.getRangeByIndexes(row, Math.max(0, column - 1), 1, column ? 2 : 1);
          const tables = near.getTables(false), pivots = near.getPivotTables(false);
          tables.load('items/id');
          pivots.load('items/name');
          return {row, cell, merges, tables, pivots};
        });
        await context.sync();
        if (worksheet.protection.protected) throw new Error('Protected sheet cannot be checked for text overflow');
        return cells.map(item => ({row: item.row, value: item.cell.values[0][0],
          formula: item.cell.formulas[0][0], type: item.cell.valueTypes[0][0],
          spill: item.cell.hasSpill, merged: !item.merges.isNullObject,
          height: item.cell.format.rowHeight,
          tables: item.tables.items.length, pivots: item.pivots.items.length}));
      });
      for (const item of items) {
        if (item.type !== 'Empty' ||
            (item.value !== '' && item.value !== null) || (item.formula !== '' && item.formula !== null) ||
            item.spill !== false || item.merged || item.tables || item.pivots ||
            !Number.isFinite(item.height) || item.height < 0) {
          throw new Error(`Cannot safely check overflow at ${sheet}!${item.row + 1}`);
        }
        targets.push({row: item.row, height: item.height});
      }
    }
    return targets;
  }

  async function sourceRows(office, sheet, candidates, baseColumn, rowCount) {
    // A merged source can draw text across several rows. Block its full height,
    // including connected merges, rather than rejecting ordinary merged titles.
    const rows = new Set(candidates);
    const checked = new Set();
    while (checked.size < rows.size) {
      const batch = [...rows].filter(row => !checked.has(row)).slice(0, 16);
      const contexts = await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet);
        const sources = batch.map(row => {
          const range = worksheet.getRangeByIndexes(row, 0, 1, baseColumn + 1);
          range.format.load('textOrientation');
          const merged = range.getMergedAreasOrNullObject();
          merged.load('areas/items/rowIndex,areas/items/rowCount');
          return {row, range, merged};
        });
        await context.sync();
        return sources.map(({row, range, merged}) => ({
          row, orientation: range.format.textOrientation,
          merges: merged.isNullObject ? [] : merged.areas.items.map(area =>
            ({first: area.rowIndex, count: area.rowCount}))
        }));
      });
      for (const source of contexts) {
        if (source.orientation !== 0) {
          const reason = source.orientation == null ? 'mixed or unavailable text orientation'
            : `text orientation ${source.orientation} degrees`;
          throw new Error(`Cannot verify overflow source context at ${sheet}!${source.row + 1}: ${reason}`);
        }
        checked.add(source.row);
        for (const merge of source.merges) {
          if (!Number.isInteger(merge.first) || !Number.isInteger(merge.count) ||
              merge.first < 0 || merge.count < 1 || merge.first + merge.count > rowCount) {
            throw new Error(`Cannot verify merged overflow source extent at ${sheet}!${source.row + 1}`);
          }
          if (merge.count > 2048) throw new Error('Merged overflow source exceeds the bounded row check');
          for (let row = merge.first; row < merge.first + merge.count; row++) {
            rows.add(row);
            if (rows.size > 2048) throw new Error('Merged overflow sources exceed the bounded row check');
          }
        }
      }
    }
    return [...rows].sort((left, right) => left - right);
  }

  function contiguous(rows) {
    const groups = [];
    for (const row of rows) {
      const previous = groups[groups.length - 1];
      if (previous && previous[previous.length - 1] + 1 === row) previous.push(row);
      else groups.push([row]);
    }
    return groups;
  }

  async function setCells(office, sheet, rows, column, value) {
    await office(async context => {
      const worksheet = context.workbook.worksheets.getItem(sheet);
      for (const group of contiguous(rows)) {
        worksheet.getRangeByIndexes(group[0], column, group.length, 1).values = group.map(() => [value]);
      }
      await context.sync();
    });
  }

  async function clearCells(office, sheet, rows, column) {
    await office(async context => {
      const worksheet = context.workbook.worksheets.getItem(sheet);
      for (const group of contiguous(rows)) {
        worksheet.getRangeByIndexes(group[0], column, group.length, 1).clear('Contents');
      }
      await context.sync();
    });
  }

  async function confirmCells(office, sheet, rows, column, expected) {
    for (let offset = 0; offset < rows.length; offset += 128) {
      const batch = rows.slice(offset, offset + 128);
      const invalid = await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet);
        const cells = batch.map(row => {
          const cell = worksheet.getRangeByIndexes(row, column, 1, 1);
          cell.load('values,formulas,valueTypes');
          return cell;
        });
        await context.sync();
        // For a literal value, Office also returns that value in formulas.
        return cells.map((cell, index) => ({row: batch[index],
          value: cell.values[0][0], formula: cell.formulas[0][0], type: cell.valueTypes[0][0]}))
          .find(cell => expected === ' '
            ? cell.value !== ' ' || cell.formula !== ' '
            : cell.type !== 'Empty' || cell.formula !== '');
      });
      if (invalid) {
        const address = `${sheet}!R${invalid.row + 1}C${column + 1}`;
        throw new Error(`Overflow check did not ${expected === ' ' ? 'write' : 'restore'} cells at ${address}: ` +
          `type=${invalid.type}, value=${JSON.stringify(invalid.value)}, formula=${JSON.stringify(invalid.formula)}`);
      }
    }
  }

  async function restoreRowHeights(office, sheet, targets, column) {
    for (let offset = 0; offset < targets.length; offset += 128) {
      await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet);
        const rows = targets.slice(offset, offset + 128).map(target => {
          const range = worksheet.getRangeByIndexes(target.row, column, 1, 1);
          range.format.load('rowHeight');
          return {range, height: target.height};
        });
        await context.sync();
        const changed = rows.filter(row => row.range.format.rowHeight !== row.height);
        if (!changed.length) return;
        for (const row of changed) row.range.format.rowHeight = row.height;
        await context.sync();
        for (const row of changed) row.range.format.load('rowHeight');
        await context.sync();
        if (changed.some(row => row.range.format.rowHeight !== row.height)) {
          throw new Error('Overflow check did not restore row heights');
        }
      });
    }
  }

  async function changedPixels(before, blocked, baseEdge, rows, geometry, tile) {
    if (before.width !== blocked.width || before.height !== blocked.height) {
      throw new Error('Worksheet geometry changed during overflow check');
    }
    const allowedBands = rows.map(row => ({
      first: Math.max(0, geometry.pixelY[row] - geometry.pixelY[tile.raw.r0] - 1),
      last: Math.min(before.height, geometry.pixelY[row + 1] - geometry.pixelY[tile.raw.r0] + 2)
    }));
    let changed = 0, rightmost = -1, band = 0;
    const rowsPerTurn = Math.max(1, Math.floor(262144 / before.width));
    for (let y = 0; y < before.height; y++) {
      if (y && y % rowsPerTurn === 0) await yieldToHost();
      while (band < allowedBands.length && allowedBands[band].last <= y) band++;
      const allowed = band < allowedBands.length && allowedBands[band].first <= y;
      for (let x = 0; x < before.width; x++) {
        const at = (y * before.width + x) * 4;
        if (before.pixels[at] === blocked.pixels[at] &&
            before.pixels[at + 1] === blocked.pixels[at + 1] &&
            before.pixels[at + 2] === blocked.pixels[at + 2] &&
            before.pixels[at + 3] === blocked.pixels[at + 3]) continue;
        if (x < baseEdge - 1 || !allowed) {
          throw new Error(`Overflow check changed pixels outside the checked area at x=${x}, y=${y}; ` +
            `source boundary=${baseEdge}, checked row=${allowed}`);
        }
        changed++;
        rightmost = Math.max(rightmost, x);
      }
    }
    return {changed, rightmost};
  }

  async function check(office, sheet, geometry, tile, baseColumn, initialBase64) {
    // Cell coordinates exclude getImage's extra outer border pixel. Including
    // that pixel here misclassifies the source edge as out-of-area changes.
    const baseEdge = Math.round(geometry.x[baseColumn + 1] * geometry.scaleX);
    let blockerColumn = baseColumn + 1;
    while (blockerColumn < tile.raw.c1 && geometry.columns[blockerColumn] === 0) blockerColumn++;
    let before, blocked, restored, written = [];
    try {
      before = await decode(initialBase64);
      // The boundary pixel itself may be a cell border rather than overflow.
      const candidates = await candidateRows(before, geometry, tile, baseEdge + 1);
      if (!candidates.length) return {retainProbe: false, rightEdge: false};
      if (blockerColumn >= tile.raw.c1) throw new Error('No visible blocker column within the image probe');
      const checkedRows = await sourceRows(office, sheet, candidates, baseColumn, geometry.pixelY.length - 1);
      const targets = await safeTargets(office, sheet, checkedRows, blockerColumn);
      written = targets.map(target => target.row);
      if (!written.length) return {retainProbe: true, rightEdge: false};
      let blockedBase64;
      try {
        await setCells(office, sheet, written, blockerColumn, ' ');
        await confirmCells(office, sheet, written, blockerColumn, ' ');
        blockedBase64 = await office(async context => {
          const image = context.workbook.worksheets.getItem(sheet).getRange(root.CaptureGeometry.address(tile.raw)).getImage();
          await context.sync();
          return image.value;
        });
      } finally {
        // Each run uses a disposable workbook. A native timeout stops further calls.
        try {
          await clearCells(office, sheet, written, blockerColumn);
          await confirmCells(office, sheet, written, blockerColumn, '');
        } finally {
          // Writing a blocker can auto-fit a row. Merged sources can extend
          // beyond this image, so restore every touched height explicitly.
          await restoreRowHeights(office, sheet, targets, blockerColumn);
        }
      }
      const restoredBase64 = await office(async context => {
        const image = context.workbook.worksheets.getItem(sheet).getRange(root.CaptureGeometry.address(tile.raw)).getImage();
        await context.sync();
        return image.value;
      });
      blocked = await decode(blockedBase64);
      restored = await decode(restoredBase64);
      if (!await sameImage(before, restored)) throw new Error('Image changed after overflow cells were restored');
      release(restored); restored = null;
      const diff = await changedPixels(before, blocked, baseEdge, written, geometry, tile);
      let lastColumn = tile.raw.c1 - 1;
      while (lastColumn > baseColumn && geometry.columns[lastColumn] === 0) lastColumn--;
      // A few blank edge pixels can be a gap between letters. Require the final
      // visible probe column to stay clear before accepting the capture boundary.
      const edge = Math.floor(geometry.x[lastColumn] * geometry.scaleX);
      return {retainProbe: diff.changed > 0, rightEdge: diff.rightmost >= edge};
    } finally {
      release(before); release(blocked); release(restored);
    }
  }

  async function crop(base64, x, y, width, height) {
    const size = CaptureGeometry.pngSize(base64);
    if (![x, y, width, height].every(Number.isInteger) || x < 0 || y < 0 || width < 1 || height < 1 ||
        x + width > size.width || y + height > size.height) throw new Error('Invalid image crop');
    // A complete tile is forwarded byte-for-byte. Reading its PNG header avoids
    // decoding a potentially 64 MP bitmap just to discover no crop is needed.
    if (x === 0 && y === 0 && width === size.width && height === size.height) return base64;
    const source = await loadImage(base64);
    const sourceWidth = source.naturalWidth, sourceHeight = source.naturalHeight;
    let canvas;
    try {
      if (![x, y, width, height].every(Number.isInteger) || x < 0 || y < 0 || width < 1 || height < 1 ||
          x + width > sourceWidth || y + height > sourceHeight) throw new Error('Invalid image crop');
      if (sourceWidth !== size.width || sourceHeight !== size.height) throw new Error('PNG dimensions differ after decoding');
      const budgetError = () => {
        const error = new Error(`Cannot encode image crop (${width} x ${height}; source ${sourceWidth} x ${sourceHeight})`);
        error.code = 'IMAGE_BUDGET';
        return error;
      };
      canvas = document.createElement('canvas');
      canvas.width = width;
      canvas.height = height;
      // Crop directly into a software canvas. A second full-size source canvas
      // needlessly doubles bitmap memory, and GPU textures impose extra limits.
      const context = canvas.getContext('2d', {willReadFrequently: true});
      if (!context) throw budgetError();
      context.drawImage(source, x, y, width, height, 0, 0, width, height);
      source.src = '';
      if (context.isContextLost?.()) throw budgetError();
      const blob = await new Promise((resolve, reject) => canvas.toBlob(value =>
        value ? resolve(value) : reject(budgetError()), 'image/png'));
      if (context.isContextLost?.()) throw budgetError();
      return await new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(reader.result.split(',')[1]);
        reader.onerror = () => reject(new Error('Cannot read image crop'));
        reader.readAsDataURL(blob);
      });
    } finally {
      source.src = '';
      if (canvas) { canvas.width = 0; canvas.height = 0; }
    }
  }

  const api = {check, crop, decode, release, sameImage};
  root.CaptureOverflow = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
