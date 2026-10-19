(function (root) {
  'use strict';

  async function decode(base64) {
    const image = new Image();
    image.src = `data:image/png;base64,${base64}`;
    await new Promise((resolve, reject) => { image.onload = resolve; image.onerror = () => reject(new Error('Cannot decode Excel PNG')); });
    const canvas = document.createElement('canvas');
    canvas.width = image.naturalWidth;
    canvas.height = image.naturalHeight;
    const context = canvas.getContext('2d', {willReadFrequently: true});
    if (!context) throw new Error('Canvas pixels are unavailable');
    context.drawImage(image, 0, 0);
    return {canvas, width: canvas.width, height: canvas.height,
      pixels: context.getImageData(0, 0, canvas.width, canvas.height).data};
  }

  function release(image) {
    if (image) { image.pixels = null; image.canvas.width = 0; image.canvas.height = 0; }
  }

  function sameImage(left, right) {
    if (left.width !== right.width || left.height !== right.height) return false;
    for (let index = 0; index < left.pixels.length; index++) {
      if (left.pixels[index] !== right.pixels[index]) return false;
    }
    return true;
  }

  function sheetRow(geometry, tile, pixelY) {
    const point = geometry.y[tile.raw.r0] + pixelY / geometry.scaleY;
    let low = tile.raw.r0, high = tile.raw.r1;
    while (low + 1 < high) {
      const middle = Math.floor((low + high) / 2);
      if (geometry.y[middle] <= point) low = middle;
      else high = middle;
    }
    return low;
  }

  function candidateRows(image, geometry, tile, baseEdge) {
    const rows = new Set();
    const width = image.width;
    if (baseEdge >= width - 2) return [];
    const firstY = tile.cropY + 1;
    const lastY = Math.min(image.height - 1, tile.cropY + tile.cropHeight - 1);
    for (let y = firstY; y < lastY; y++) {
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
          tables: item.tables.items.length, pivots: item.pivots.items.length}));
      });
      for (const item of items) {
        if (item.type !== 'Empty' ||
            (item.value !== '' && item.value !== null) || (item.formula !== '' && item.formula !== null) ||
            item.spill !== false || item.merged || item.tables || item.pivots) {
          throw new Error(`Cannot safely check overflow at ${sheet}!${item.row + 1}`);
        }
        targets.push(item.row);
      }
    }
    return targets;
  }

  async function safeSource(office, sheet, rows, baseColumn) {
    for (let offset = 0; offset < rows.length; offset += 16) {
      const batch = rows.slice(offset, offset + 16);
      const safe = await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet);
        const sources = batch.map(row => {
          const range = worksheet.getRangeByIndexes(row, 0, 1, baseColumn + 1);
          range.format.load('textOrientation');
          const merged = range.getMergedAreasOrNullObject();
          merged.load('areas/items/rowCount');
          return {range, merged};
        });
        await context.sync();
        return sources.every(({range, merged}) => range.format.textOrientation === 0 &&
          (merged.isNullObject || merged.areas.items.every(area => area.rowCount === 1)));
      });
      if (!safe) throw new Error(`Cannot verify overflow source context at ${sheet}!${batch[0] + 1}`);
    }
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

  async function confirmCells(office, sheet, rows, column, expected) {
    for (let offset = 0; offset < rows.length; offset += 128) {
      const batch = rows.slice(offset, offset + 128);
      const valid = await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet);
        const cells = batch.map(row => {
          const cell = worksheet.getRangeByIndexes(row, column, 1, 1);
          cell.load('values,formulas,valueTypes');
          return cell;
        });
        await context.sync();
        // For a literal value, Office also returns that value in formulas.
        return cells.every(cell => expected === ' '
          ? cell.values[0][0] === ' ' && cell.formulas[0][0] === ' '
          : cell.valueTypes[0][0] === 'Empty' && cell.formulas[0][0] === '');
      });
      if (!valid) throw new Error(`Overflow check did not ${expected === ' ' ? 'write' : 'restore'} cells`);
    }
  }

  function changedPixels(before, blocked, baseEdge, rows, geometry, tile) {
    if (before.width !== blocked.width || before.height !== blocked.height) {
      throw new Error('Worksheet geometry changed during overflow check');
    }
    const allowedBands = rows.map(row => ({
      first: Math.max(0, Math.floor((geometry.y[row] - geometry.y[tile.raw.r0]) * geometry.scaleY) - 1),
      last: Math.min(before.height, Math.ceil((geometry.y[row + 1] - geometry.y[tile.raw.r0]) * geometry.scaleY) + 2)
    }));
    let changed = 0, rightmost = -1;
    for (let y = 0; y < before.height; y++) {
      for (let x = 0; x < before.width; x++) {
        const at = (y * before.width + x) * 4;
        if (before.pixels[at] === blocked.pixels[at] &&
            before.pixels[at + 1] === blocked.pixels[at + 1] &&
            before.pixels[at + 2] === blocked.pixels[at + 2] &&
            before.pixels[at + 3] === blocked.pixels[at + 3]) continue;
        if (x < baseEdge - 1 || !allowedBands.some(band => y >= band.first && y < band.last)) {
          throw new Error('Overflow check changed pixels outside the checked area');
        }
        changed++;
        rightmost = Math.max(rightmost, x);
      }
    }
    return {changed, rightmost};
  }

  async function check(office, sheet, geometry, tile, baseColumn, initialBase64) {
    const baseEdge = Math.round(geometry.x[baseColumn + 1] * geometry.scaleX) + 1;
    let blockerColumn = baseColumn + 1;
    while (blockerColumn < tile.raw.c1 && geometry.columns[blockerColumn] === 0) blockerColumn++;
    let before, blocked, restored, written = [];
    try {
      before = await decode(initialBase64);
      const candidates = candidateRows(before, geometry, tile, baseEdge);
      if (!candidates.length) return {retainProbe: false, rightEdge: false};
      if (blockerColumn >= tile.raw.c1) throw new Error('No visible blocker column within the image probe');
      written = await safeTargets(office, sheet, candidates, blockerColumn);
      if (!written.length) return {retainProbe: true, rightEdge: false};
      await safeSource(office, sheet, written, baseColumn);
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
        await setCells(office, sheet, written, blockerColumn, '');
        await confirmCells(office, sheet, written, blockerColumn, '');
      }
      const restoredBase64 = await office(async context => {
        const image = context.workbook.worksheets.getItem(sheet).getRange(root.CaptureGeometry.address(tile.raw)).getImage();
        await context.sync();
        return image.value;
      });
      blocked = await decode(blockedBase64);
      restored = await decode(restoredBase64);
      if (!sameImage(before, restored)) throw new Error('Image changed after overflow cells were restored');
      const diff = changedPixels(before, blocked, baseEdge, written, geometry, tile);
      return {retainProbe: diff.changed > 0, rightEdge: diff.rightmost >= before.width - 3};
    } finally {
      release(before); release(blocked); release(restored);
    }
  }

  async function crop(base64, x, y, width, height) {
    const source = await decode(base64);
    try {
      if (![x, y, width, height].every(Number.isInteger) || x < 0 || y < 0 || width < 1 || height < 1 ||
          x + width > source.width || y + height > source.height) throw new Error('Invalid image crop');
      const canvas = document.createElement('canvas');
      canvas.width = width;
      canvas.height = height;
      canvas.getContext('2d').drawImage(source.canvas, x, y, width, height, 0, 0, width, height);
      return canvas.toDataURL('image/png').split(',')[1];
    } finally { release(source); }
  }

  const api = {check, crop, decode, release, sameImage};
  root.CaptureOverflow = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
