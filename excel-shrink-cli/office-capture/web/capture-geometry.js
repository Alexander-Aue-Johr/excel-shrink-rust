(function (root) {
  'use strict';

  const MAX_ROWS = 1048576;
  const MAX_COLUMNS = 16384;
  const MAX_PIXELS = 32000000;
  const MAX_EDGE = 30000;
  const MAX_TILE_ROWS = 8192;
  const BATCH = 384;

  function budgetError(message) {
    const error = new Error(message);
    error.code = 'IMAGE_BUDGET';
    return error;
  }

  function columnName(index) {
    let name = '';
    for (let n = index + 1; n; n = Math.floor((n - 1) / 26)) {
      name = String.fromCharCode(65 + (n - 1) % 26) + name;
    }
    return name;
  }

  function address(rect) {
    return `${columnName(rect.c0)}${rect.r0 + 1}:${columnName(rect.c1 - 1)}${rect.r1}`;
  }

  function pngSize(base64) {
    if (typeof base64 !== 'string' || base64.length < 60) throw new Error('Excel returned an empty image');
    const head = atob(base64.slice(0, 44));
    const tail = atob(base64.slice(-32));
    const signature = [137, 80, 78, 71, 13, 10, 26, 10];
    if (!signature.every((byte, index) => head.charCodeAt(index) === byte) || head.slice(12, 16) !== 'IHDR' || tail.slice(-8, -4) !== 'IEND') {
      throw new Error('Excel returned an invalid or truncated PNG');
    }
    const read = offset => (head.charCodeAt(offset) * 16777216 + (head.charCodeAt(offset + 1) << 16) +
      (head.charCodeAt(offset + 2) << 8) + head.charCodeAt(offset + 3));
    const width = read(16), height = read(20);
    if (width < 1 || height < 1 || width > MAX_EDGE || height > MAX_EDGE || width * height > MAX_PIXELS) {
      throw budgetError(`Image exceeds the capture budget (${width} × ${height})`);
    }
    return {width, height};
  }

  function scaleFor(samples, axis) {
    const points = samples.map(sample => [sample[axis], sample[axis === 'width' ? 'pngWidth' : 'pngHeight']]);
    const lengths = points.map(point => point[0]);
    if (new Set(lengths).size < 3 || Math.max(...lengths) - Math.min(...lengths) < 8) {
      throw new Error(`Insufficient ${axis} variation for image calibration`);
    }
    const lower = Math.max(...points.map(([length, pixels]) => (pixels - 1.5) / length));
    const upper = Math.min(...points.map(([length, pixels]) => (pixels - 0.5) / length));
    if (!(upper > lower)) throw new Error(`Inconsistent ${axis} image calibration`);
    let scale = null;
    for (let denominator = 1; denominator <= 512 && scale === null; denominator++) {
      const candidate = Math.ceil(lower * denominator - 1e-10) / denominator;
      if (candidate >= lower - 1e-12 && candidate < upper &&
          points.every(([length, pixels]) => Math.round(length * candidate) + 1 === pixels)) scale = candidate;
    }
    if (scale === null) scale = (lower + upper) / 2;
    if (scale < .25 || scale > 8 || !points.every(([length, pixels]) => Math.round(length * scale) + 1 === pixels)) {
      throw new Error(`Image calibration failed for ${axis}`);
    }
    return scale;
  }

  function prefix(sizes) {
    const values = [0];
    for (const size of sizes) values.push(values[values.length - 1] + size);
    return values;
  }

  async function readHeightRuns(office, sheet, bottomRow) {
    if (!Number.isInteger(bottomRow) || bottomRow < 0 || bottomRow >= MAX_ROWS) {
      throw new Error(`Invalid row boundary for ${sheet}`);
    }
    const pending = [];
    for (let start = 0; start <= bottomRow; start += 8192) {
      pending.push({start, end: Math.min(bottomRow + 1, start + 8192)});
    }
    const measured = [];
    let cursor = 0;
    while (cursor < pending.length) {
      if (pending.length > 250000) throw new Error(`Row heights exceed the request budget for ${sheet}`);
      const batch = pending.slice(cursor, cursor + 64);
      cursor += batch.length;
      const values = await office(async context => {
        const worksheet = context.workbook.worksheets.getItem(sheet);
        const ranges = batch.map(span => {
          const range = worksheet.getRangeByIndexes(span.start, 0, span.end - span.start, 1);
          range.load('height,rowHidden,rowCount');
          range.format.load('rowHeight');
          return range;
        });
        await context.sync();
        return ranges.map(range => ({height: range.height, hidden: range.rowHidden,
          rowHeight: range.format.rowHeight, rowCount: range.rowCount}));
      });
      for (let index = 0; index < batch.length; index++) {
        const span = batch[index], item = values[index], count = span.end - span.start;
        const visible = typeof item.height === 'number' && Number.isFinite(item.height) && item.height >= 0;
        const hidden = item.hidden;
        const uniform = typeof item.rowHeight === 'number' && Number.isFinite(item.rowHeight) && item.rowHeight >= 0;
        if (item.rowCount !== count) throw new Error(`Row range changed while checking ${sheet}`);
        if (count === 1) {
          if (!visible || typeof hidden !== 'boolean') throw new Error(`Ambiguous row height at ${sheet}!${span.start + 1}`);
          measured.push({start_row: span.start + 1, end_row: span.end,
            height: hidden ? 0 : item.height});
          continue;
        }
        const expected = hidden === true ? 0 : Number(item.rowHeight) * count;
        if (visible && typeof hidden === 'boolean' && (hidden || uniform) &&
            Math.abs(item.height - expected) <= Math.max(.01, Math.abs(expected) * 1e-9)) {
          measured.push({start_row: span.start + 1, end_row: span.end,
            height: hidden ? 0 : item.rowHeight});
        } else {
          const middle = span.start + Math.floor(count / 2);
          pending.push({start: span.start, end: middle}, {start: middle, end: span.end});
        }
      }
    }
    measured.sort((left, right) => left.start_row - right.start_row);
    const runs = [];
    for (const item of measured) {
      const previous = runs[runs.length - 1];
      if (previous && previous.end_row + 1 === item.start_row && previous.height === item.height) {
        previous.end_row = item.end_row;
      } else runs.push(item);
    }
    if (!runs.length || runs[0].start_row !== 1 || runs[runs.length - 1].end_row !== bottomRow + 1 ||
        runs.some((run, index) => index && runs[index - 1].end_row + 1 !== run.start_row)) {
      throw new Error(`Row height coverage is incomplete for ${sheet}`);
    }
    return runs;
  }

  class SheetGeometry {
    constructor(office, sheet, bottomRow, baseColumn, tileMegapixels = 32) {
      if (tileMegapixels !== 16 && tileMegapixels !== 32) throw new Error('Invalid tile megapixel budget');
      this.office = office;
      this.sheet = sheet;
      this.bottomRow = bottomRow;
      this.baseColumn = baseColumn;
      this.maxPixels = tileMegapixels * 1000000;
      this.rows = [];
      this.columns = [];
    }

    async readSizes(axis, start, count) {
      const sizes = [];
      for (let offset = 0; offset < count; offset += BATCH) {
        const length = Math.min(BATCH, count - offset);
        const values = await this.office(async context => {
          const worksheet = context.workbook.worksheets.getItem(this.sheet);
          const ranges = Array.from({length}, (_, index) => {
            const position = start + offset + index;
            const range = axis === 'rows'
              ? worksheet.getRangeByIndexes(position, this.columnAnchor ?? 0, 1, 1)
              : worksheet.getRangeByIndexes(0, position, 1, 1);
            range.load(axis === 'rows' ? 'height,rowHidden' : 'width,columnHidden');
            return range;
          });
          await context.sync();
          return ranges.map(range => axis === 'rows'
            ? {size: Number(range.height), hidden: range.rowHidden}
            : {size: Number(range.width), hidden: range.columnHidden});
        });
        for (const value of values) {
          if (!Number.isFinite(value.size) || value.size < 0 || typeof value.hidden !== 'boolean') {
            throw new Error(`Invalid worksheet ${axis} geometry`);
          }
          sizes.push(value.hidden ? 0 : value.size);
        }
      }
      return sizes;
    }

    async read(endColumn) {
      if (!Number.isInteger(this.bottomRow) || this.bottomRow < 0 || this.bottomRow >= MAX_ROWS ||
          !Number.isInteger(endColumn) || endColumn < 0 || endColumn >= MAX_COLUMNS) {
        throw new Error('Worksheet capture bounds are outside the supported grid');
      }
      this.columns = await this.readSizes('columns', 0, endColumn + 1);
      this.columnAnchor = Math.max(0, this.columns.findIndex(size => size > 0));
      // Individual visible heights define image geometry; aggregate heights can
      // accumulate fractional row sizes differently from the rendered cells.
      this.rows = await this.readSizes('rows', 0, this.bottomRow + 1);
      this.x = prefix(this.columns);
      this.y = prefix(this.rows);
    }

    async calibrate() {
      const smallestVisible = sizes => {
        const first = sizes.findIndex(size => size > 0);
        let best = first;
        for (let index = first + 1; index < Math.min(sizes.length, first + 64); index++) {
          if (sizes[index] > 0 && sizes[index] < sizes[best]) best = index;
        }
        return best;
      };
      const firstRow = smallestVisible(this.rows);
      const firstColumn = smallestVisible(this.columns);
      if (firstRow < 0 || firstColumn < 0) throw new Error('Worksheet has no visible capture geometry');
      const rows = this.rows.slice(firstRow, firstRow + 32);
      const columns = this.columns.slice(firstColumn, firstColumn + 32);
      rows.push(...await this.readSizes('rows', firstRow + rows.length,
        Math.min(32 - rows.length, MAX_ROWS - firstRow - rows.length)));
      columns.push(...await this.readSizes('columns', firstColumn + columns.length,
        Math.min(32 - columns.length, MAX_COLUMNS - firstColumn - columns.length)));
      const x = prefix(columns), y = prefix(rows);
      // Vary each axis separately so tall rows and wide columns still admit
      // small controls inside the image budget.
      const candidates = [[1,1], [2,1], [4,1], [8,1], [16,1], [32,1],
        [1,2], [1,4], [1,8], [1,16], [1,32]];
      const samples = [];
      for (const [rowCount, columnCount] of candidates) {
        if (rowCount > rows.length || columnCount > columns.length) continue;
        const sample = {width: x[columnCount], height: y[rowCount]};
        if (sample.width <= 0 || sample.height <= 0 ||
            (sample.width * 8 + 1) > 8192 || (sample.height * 8 + 1) > 8192 ||
            (sample.width * 8 + 1) * (sample.height * 8 + 1) > 4000000) continue;
        const base64 = await this.office(async context => {
          const image = context.workbook.worksheets.getItem(this.sheet)
            .getRangeByIndexes(firstRow, firstColumn, rowCount, columnCount).getImage();
          await context.sync();
          return image.value;
        });
        const png = pngSize(base64);
        samples.push({...sample, pngWidth: png.width, pngHeight: png.height});
      }
      if (samples.length < 3) throw new Error('No safe image calibration controls were available');
      const widthVariation = new Set(samples.map(sample => sample.width)).size;
      this.scaleY = scaleFor(samples, 'height');
      if (widthVariation >= 3) {
        this.scaleX = scaleFor(samples, 'width');
        this.fixedWidth = null;
      } else {
        const widthPoints = this.x[this.x.length - 1];
        if (widthPoints <= 0 || widthPoints * 8 + 1 > MAX_EDGE) throw budgetError('Full-width image calibration exceeds the edge budget');
        const base64 = await this.office(async context => {
          const image = context.workbook.worksheets.getItem(this.sheet)
            .getRangeByIndexes(firstRow, 0, 1, this.columns.length).getImage();
          await context.sync();
          return image.value;
        });
        this.fixedWidth = pngSize(base64).width;
        this.scaleX = null;
      }
    }

    imageWidth(endColumn) {
      if (this.fixedWidth !== null) {
        if (endColumn !== this.columns.length - 1) throw new Error('Fixed-width capture cannot crop columns');
        return this.fixedWidth;
      }
      return Math.round(this.x[endColumn + 1] * this.scaleX) + 1;
    }

    imageHeight(start, end) { return Math.round((this.y[end] - this.y[start]) * this.scaleY) + 1; }

    halo(start, end) {
      const needed = 2 / this.scaleY;
      let first = start, last = end;
      if (this.y[start] - this.y[0] > 0) {
        const target = this.y[start] - needed;
        let low = 0, high = start;
        while (low < high) {
          const middle = Math.ceil((low + high) / 2);
          if (this.y[middle] <= target) low = middle;
          else high = middle - 1;
        }
        first = low;
      }
      if (this.y[this.rows.length] - this.y[end] > 0) {
        const target = this.y[end] + needed;
        let low = end, high = this.rows.length;
        while (low < high) {
          const middle = Math.floor((low + high) / 2);
          if (this.y[middle] >= target) high = middle;
          else low = middle + 1;
        }
        last = low;
      }
      if (last - first > MAX_TILE_ROWS + 2048) throw budgetError('Visible context exceeds the tile row budget');
      return {first, last};
    }

    tiles(endColumn, targets) {
      const width = this.imageWidth(endColumn);
      if (width > MAX_EDGE || width < 2) throw budgetError(`Full-width capture exceeds the image edge budget (${width} px)`);
      const maxHeight = Math.min(MAX_EDGE, Math.floor(this.maxPixels / width));
      const spans = [];
      if (targets) {
        for (const row of targets) {
          const start = Math.max(0, row - 2), end = Math.min(this.rows.length, row + 1);
          const previous = spans[spans.length - 1];
          if (previous && start <= previous.end) previous.end = Math.max(previous.end, end);
          else spans.push({start, end});
        }
      } else spans.push({start: 0, end: this.rows.length});
      const tiles = [];
      for (const span of spans) {
        let start = span.start;
        while (start < span.end) {
          let low = start + 1, high = Math.min(span.end, start + MAX_TILE_ROWS) + 1;
          while (low + 1 < high) {
            const middle = Math.floor((low + high) / 2);
            const halo = this.halo(start, middle);
            if (this.imageHeight(halo.first, halo.last) <= maxHeight) low = middle;
            else high = middle;
          }
          const end = low;
          const halo = this.halo(start, end);
          if (this.imageHeight(halo.first, halo.last) > maxHeight) throw budgetError(`Row ${start + 1} exceeds the image budget`);
          const raw = {r0: halo.first, r1: halo.last, c0: 0, c1: endColumn + 1};
          const core = {r0: start, r1: end, c0: 0, c1: endColumn + 1};
          const cropY = Math.round((this.y[start] - this.y[halo.first]) * this.scaleY);
          const cropHeight = this.imageHeight(start, end);
          tiles.push({raw, core, cropY, cropHeight, width, expectedHeight: this.imageHeight(halo.first, halo.last)});
          if (tiles.length > 4096) throw budgetError('Worksheet exceeds the tile count budget');
          start = end;
        }
      }
      return tiles;
    }
  }

  const api = {SheetGeometry, readHeightRuns, address, pngSize, MAX_PIXELS, MAX_EDGE};
  root.CaptureGeometry = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
