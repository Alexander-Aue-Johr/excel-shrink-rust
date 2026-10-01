/* global Excel, ExperimentUi, ExperimentEvents */
'use strict';

(async function () {
  const MAX_ROWS = 1048576;
  const MAX_COLUMNS = 16384;

  function fixed(value) {
    return Number(Number(value).toFixed(6));
  }

  function columnName(indexZeroBased) {
    let n = Number(indexZeroBased) + 1;
    let name = '';
    while (n > 0) {
      const rem = (n - 1) % 26;
      name = String.fromCharCode(65 + rem) + name;
      n = Math.floor((n - 1) / 26);
    }
    return name;
  }

  function addressFromBounds(rowIndex, columnIndex, rowCount, columnCount) {
    const firstRow = Number(rowIndex) + 1;
    const firstColumn = Number(columnIndex);
    const lastRow = firstRow + Number(rowCount) - 1;
    const lastColumn = firstColumn + Number(columnCount) - 1;
    return `${columnName(firstColumn)}${firstRow}:${columnName(lastColumn)}${lastRow}`;
  }

  function addressA1To(rowZeroBased, columnZeroBased) {
    const row = Math.max(0, Math.min(MAX_ROWS - 1, Number(rowZeroBased)));
    const col = Math.max(0, Math.min(MAX_COLUMNS - 1, Number(columnZeroBased)));
    return `A1:${columnName(col)}${row + 1}`;
  }

  function visibleWorksheet(sheet) {
    return String(sheet.visibility || '').toLowerCase() === 'visible';
  }

  async function getWorkbookAndVisibleSheets() {
    return Excel.run(async (context) => {
      const workbook = context.workbook;
      const sheets = workbook.worksheets;
      workbook.load('name');
      sheets.load('items/name,items/visibility');
      await context.sync();

      return {
        workbookName: workbook.name,
        sheets: sheets.items
          .filter(visibleWorksheet)
          .map((sheet) => ({ name: sheet.name }))
      };
    });
  }

  async function resolveUsedRanges(sheetNames, valuesOnly) {
    return Excel.run(async (context) => {
      const workbook = context.workbook;
      const entries = [];

      for (const sheetName of sheetNames) {
        const sheet = workbook.worksheets.getItem(sheetName);
        const used = sheet.getUsedRangeOrNullObject(Boolean(valuesOnly));
        used.load('address,rowIndex,columnIndex,rowCount,columnCount,left,top,width,height');
        entries.push({ sheetName, used });
      }

      const started = performance.now();
      await context.sync();
      const elapsed = performance.now() - started;

      return {
        elapsed_ms: elapsed,
        entries: entries.map((entry) => {
          if (entry.used.isNullObject) {
            return {
              sheet: entry.sheetName,
              address: 'A1:A1',
              rowIndex: 0,
              columnIndex: 0,
              rowCount: 1,
              columnCount: 1,
              left: 0,
              top: 0,
              width: 0,
              height: 0,
              empty: true
            };
          }

          return {
            sheet: entry.sheetName,
            address: entry.used.address.split('!').pop(),
            rowIndex: Number(entry.used.rowIndex),
            columnIndex: Number(entry.used.columnIndex),
            rowCount: Number(entry.used.rowCount),
            columnCount: Number(entry.used.columnCount),
            left: Number(entry.used.left),
            top: Number(entry.used.top),
            width: Number(entry.used.width),
            height: Number(entry.used.height),
            empty: false
          };
        })
      };
    });
  }

  async function collectContentAndShapeBounds(sheetNames) {
    return Excel.run(async (context) => {
      const workbook = context.workbook;
      const entries = [];

      for (const sheetName of sheetNames) {
        const sheet = workbook.worksheets.getItem(sheetName);
        const content = sheet.getUsedRangeOrNullObject(true);
        content.load('address,rowIndex,columnIndex,rowCount,columnCount,left,top,width,height');

        const shapes = sheet.shapes;
        shapes.load(
          'items/left,items/top,items/width,items/height,' +
          'items/rotation,items/visible'
        );

        entries.push({ sheetName, content, shapes });
      }

      const started = performance.now();
      await context.sync();
      const elapsed = performance.now() - started;

      return {
        elapsed_ms: elapsed,
        entries: entries.map((entry) => {
          const content = entry.content.isNullObject
            ? {
                rowIndex: 0,
                columnIndex: 0,
                rowCount: 1,
                columnCount: 1,
                left: 0,
                top: 0,
                width: 0,
                height: 0,
                empty: true
              }
            : {
                rowIndex: Number(entry.content.rowIndex),
                columnIndex: Number(entry.content.columnIndex),
                rowCount: Number(entry.content.rowCount),
                columnCount: Number(entry.content.columnCount),
                left: Number(entry.content.left),
                top: Number(entry.content.top),
                width: Number(entry.content.width),
                height: Number(entry.content.height),
                empty: false
              };

          let shapeRightPoints = 0;
          let shapeBottomPoints = 0;
          let visibleShapeCount = 0;

          for (const shape of entry.shapes.items) {
            if (!shape.visible) continue;
            visibleShapeCount += 1;

            const angle = Number(shape.rotation) * Math.PI / 180.0;
            const width = Number(shape.width);
            const height = Number(shape.height);
            const rotatedWidth =
              Math.abs(width * Math.cos(angle)) +
              Math.abs(height * Math.sin(angle));
            const rotatedHeight =
              Math.abs(width * Math.sin(angle)) +
              Math.abs(height * Math.cos(angle));

            shapeRightPoints = Math.max(
              shapeRightPoints,
              Number(shape.left) + width / 2 + rotatedWidth / 2
            );
            shapeBottomPoints = Math.max(
              shapeBottomPoints,
              Number(shape.top) + height / 2 + rotatedHeight / 2
            );
          }

          const contentRightColumn =
            content.columnIndex + content.columnCount - 1;
          const contentBottomRow =
            content.rowIndex + content.rowCount - 1;
          const contentRightPoints = content.left + content.width;
          const contentBottomPoints = content.top + content.height;

          return {
            sheet: entry.sheetName,
            content,
            contentRightColumn,
            contentBottomRow,
            contentRightPoints,
            contentBottomPoints,
            shapeRightPoints,
            shapeBottomPoints,
            visibleShapeCount,
            rightColumn: contentRightColumn,
            bottomRow: contentBottomRow
          };
        })
      };
    });
  }

  async function mapPointBoundsToGrid(entries) {
    const horizontal = entries
      .filter((entry) => entry.shapeRightPoints > entry.contentRightPoints + 0.01)
      .map((entry) => ({
        entry,
        low: Math.max(0, entry.contentRightColumn),
        high: MAX_COLUMNS - 1,
        target: entry.shapeRightPoints
      }));

    const vertical = entries
      .filter((entry) => entry.shapeBottomPoints > entry.contentBottomPoints + 0.01)
      .map((entry) => ({
        entry,
        low: Math.max(0, entry.contentBottomRow),
        high: MAX_ROWS - 1,
        target: entry.shapeBottomPoints
      }));

    const started = performance.now();

    while (
      horizontal.some((item) => item.low < item.high) ||
      vertical.some((item) => item.low < item.high)
    ) {
      await Excel.run(async (context) => {
        const workbook = context.workbook;
        const probes = [];

        for (const item of horizontal) {
          if (item.low >= item.high) continue;
          const mid = Math.floor((item.low + item.high) / 2);
          const cell = workbook.worksheets
            .getItem(item.entry.sheet)
            .getRangeByIndexes(0, mid, 1, 1);
          cell.load('left,width');
          probes.push({ axis: 'x', item, mid, cell });
        }

        for (const item of vertical) {
          if (item.low >= item.high) continue;
          const mid = Math.floor((item.low + item.high) / 2);
          const cell = workbook.worksheets
            .getItem(item.entry.sheet)
            .getRangeByIndexes(mid, 0, 1, 1);
          cell.load('top,height');
          probes.push({ axis: 'y', item, mid, cell });
        }

        await context.sync();

        for (const probe of probes) {
          const edge = probe.axis === 'x'
            ? Number(probe.cell.left) + Number(probe.cell.width)
            : Number(probe.cell.top) + Number(probe.cell.height);

          if (edge + 0.01 >= probe.item.target) {
            probe.item.high = probe.mid;
          } else {
            probe.item.low = probe.mid + 1;
          }
        }
      });
    }

    for (const item of horizontal) {
      item.entry.rightColumn = Math.max(item.entry.rightColumn, item.low);
    }
    for (const item of vertical) {
      item.entry.bottomRow = Math.max(item.entry.bottomRow, item.low);
    }

    return performance.now() - started;
  }

  async function probeEndColumnByPixelWidth(sheetName, startColumn, targetPoints) {
    let low = startColumn;
    let high = MAX_COLUMNS - 1;

    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      const width = await Excel.run(async (context) => {
        const range = context.workbook.worksheets
          .getItem(sheetName)
          .getRangeByIndexes(0, startColumn, 1, mid - startColumn + 1);
        range.load('width');
        await context.sync();
        return Number(range.width);
      });

      if (width >= targetPoints) high = mid;
      else low = mid + 1;
    }

    return low;
  }

  async function probeEndRowByPixelHeight(sheetName, startRow, targetPoints) {
    let low = startRow;
    let high = MAX_ROWS - 1;

    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      const height = await Excel.run(async (context) => {
        const range = context.workbook.worksheets
          .getItem(sheetName)
          .getRangeByIndexes(startRow, 0, mid - startRow + 1, 1);
        range.load('height');
        await context.sync();
        return Number(range.height);
      });

      if (height >= targetPoints) high = mid;
      else low = mid + 1;
    }

    return low;
  }

  async function getColumnBoundaries(sheetName, startColumn, endColumn) {
    return Excel.run(async (context) => {
      const sheet = context.workbook.worksheets.getItem(sheetName);
      const ranges = [];
      for (let col = startColumn; col <= endColumn; col += 1) {
        const cell = sheet.getRangeByIndexes(0, col, 1, 1);
        cell.load('width');
        ranges.push(cell);
      }
      await context.sync();
      let cumulative = 0;
      return ranges.map((range) => {
        cumulative += Number(range.width);
        return cumulative;
      });
    });
  }

  async function getRowBoundaries(sheetName, startRow, endRow) {
    return Excel.run(async (context) => {
      const sheet = context.workbook.worksheets.getItem(sheetName);
      const ranges = [];
      for (let row = startRow; row <= endRow; row += 1) {
        const cell = sheet.getRangeByIndexes(row, 0, 1, 1);
        cell.load('height');
        ranges.push(cell);
      }
      await context.sync();
      let cumulative = 0;
      return ranges.map((range) => {
        cumulative += Number(range.height);
        return cumulative;
      });
    });
  }

  function officeErrorDetail(error) {
    const message = error && error.message ? error.message : String(error);
    const code = error && error.code ? String(error.code) : '';
    let debugInfo = '';

    try {
      if (error && error.debugInfo) {
        debugInfo = JSON.stringify(error.debugInfo);
      }
    } catch (_) {
    }

    return [
      message,
      code ? `code=${code}` : '',
      debugInfo ? `debugInfo=${debugInfo}` : ''
    ].filter(Boolean).join(' | ');
  }

  async function syncStage(context, stage) {
    try {
      await context.sync();
    } catch (error) {
      throw new Error(`${stage}: ${officeErrorDetail(error)}`);
    }
  }

  async function captureProbePair(
    workbookName,
    entry,
    axis,
    startIndex,
    endIndex,
    address,
    boundariesPoints,
    totalPoints
  ) {
    return Excel.run(async (context) => {
      const sheet = context.workbook.worksheets.getItem(entry.sheet);
      const probe = sheet.getRange(address);

      // Keep the full probe-range emptiness guard: the XOR strategy is only
      // meaningful when the area outside the content boundary is actually empty.
      const constants = probe.getSpecialCellsOrNullObject(
        Excel.SpecialCellType.constants
      );
      const formulas = probe.getSpecialCellsOrNullObject(
        Excel.SpecialCellType.formulas
      );

      await syncStage(context, `probe emptiness check (${entry.sheet}, ${axis})`);

      if (!constants.isNullObject || !formulas.isNullObject) {
        return {
          skipped: true,
          reason: 'probe-range-has-content'
        };
      }

      // The old implementation copied a space into *every cell* of the complete
      // screenshot-sized probe and then cleared that complete range again. On
      // large worksheets this can touch hundreds of thousands of cells and Excel
      // may fail the final context.sync() with a generic RichApi internal error.
      //
      // A blocker is only needed directly at the content boundary:
      //   horizontal overflow -> first column to the right
      //   vertical overflow   -> first row below
      // The image still covers the complete probe range, so the measured XOR
      // extent is unchanged while the mutation is dramatically smaller.
      const blocker = axis === 'horizontal'
        ? sheet.getRangeByIndexes(
            0,
            startIndex,
            Math.max(1, Number(entry.contentBottomRow) + 1),
            1
          )
        : sheet.getRangeByIndexes(
            startIndex,
            0,
            1,
            Math.max(1, Number(entry.contentRightColumn) + 1)
          );

      const before = probe.getImage();
      await syncStage(context, `probe before image (${entry.sheet}, ${axis})`);

      const seed = blocker.getCell(0, 0);
      seed.values = [[' ']];
      blocker.copyFrom(seed, Excel.RangeCopyType.values);

      let blockerWritten = false;
      let after;

      try {
        await syncStage(context, `write overflow blocker (${entry.sheet}, ${axis})`);
        blockerWritten = true;

        after = probe.getImage();
        await syncStage(context, `probe after image (${entry.sheet}, ${axis})`);
      } finally {
        if (blockerWritten) {
          try {
            // Range.clear() has produced a reproducible GeneralException on
            // otherwise writable empty blocker ranges in desktop Excel.
            //
            // Restore through the same value-copy path that successfully wrote
            // the blocker: an empty string clears the seed cell, and
            // skipBlanks=false makes copyFrom propagate that blank value across
            // the complete blocker range.
            seed.values = [['']];
            await syncStage(
              context,
              `clear overflow blocker seed (${entry.sheet}, ${axis})`
            );

            blocker.copyFrom(
              seed,
              Excel.RangeCopyType.values,
              false,
              false
            );
            await syncStage(
              context,
              `restore overflow blocker (${entry.sheet}, ${axis})`
            );
          } catch (cleanupError) {
            throw cleanupError;
          }
        }
      }

      const response = await fetch('/probe-batch', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          workbook: workbookName,
          sheet: entry.sheet,
          axis,
          start_index: startIndex,
          end_index: endIndex,
          address,
          before_base64: before.value,
          after_base64: after.value,
          boundaries_points: boundariesPoints,
          total_points: totalPoints
        })
      });

      if (!response.ok) {
        throw new Error(`Probe upload failed: ${await response.text()}`);
      }

      return { skipped: false };
    });
  }

  async function waitForProbeResults(workbookName, expected) {
    for (;;) {
      const response = await fetch(
        `/probe-status?workbook=${encodeURIComponent(workbookName)}`
      );
      if (!response.ok) throw new Error(await response.text());
      const status = await response.json();
      ExperimentUi.showAnalysis(
        Number(status.completed || 0),
        expected,
        status.done
          ? 'Overflow analysis complete.'
          : 'The host is analysing XOR probe pairs in parallel.'
      );

      if (status.done) return status.results || [];
      await new Promise((resolve) => window.setTimeout(resolve, 150));
    }
  }

  async function runOverflowPlanning(workbookName, entries, config) {
    const horizontal = config.strategy === 'D' || config.strategy === 'E';
    const vertical = config.strategy === 'E';
    let submitted = 0;

    ExperimentUi.setPhase('Capturing overflow probes');

    for (let index = 0; index < entries.length; index += 1) {
      const entry = entries[index];
      ExperimentUi.showAnalysis(
        submitted,
        entries.length * (horizontal ? 1 : 0) +
          entries.length * (vertical ? 1 : 0),
        `Probe ${index + 1} / ${entries.length}: ${entry.sheet}`
      );

      if (horizontal && entry.rightColumn < MAX_COLUMNS - 1) {
        const startColumn = entry.rightColumn + 1;
        const targetPoints = Number(config.overflow_probe_width_px || 1920) * 72 / 96;
        const endColumn = await probeEndColumnByPixelWidth(
          entry.sheet,
          startColumn,
          targetPoints
        );
        const boundaries = await getColumnBoundaries(
          entry.sheet,
          startColumn,
          endColumn
        );
        const address =
          `${columnName(startColumn)}1:` +
          `${columnName(endColumn)}${Math.max(1, entry.contentBottomRow + 1)}`;

        const result = await captureProbePair(
          workbookName,
          entry,
          'horizontal',
          startColumn,
          endColumn,
          address,
          boundaries,
          boundaries.length ? boundaries[boundaries.length - 1] : 0
        );

        if (!result.skipped) submitted += 1;
      }

      if (vertical && entry.bottomRow < MAX_ROWS - 1) {
        const startRow = entry.bottomRow + 1;
        const targetPoints = Number(config.overflow_probe_height_px || 1080) * 72 / 96;
        const endRow = await probeEndRowByPixelHeight(
          entry.sheet,
          startRow,
          targetPoints
        );
        const boundaries = await getRowBoundaries(
          entry.sheet,
          startRow,
          endRow
        );
        const lastColumn = Math.max(0, entry.contentRightColumn);
        const address =
          `A${startRow + 1}:${columnName(lastColumn)}${endRow + 1}`;

        const result = await captureProbePair(
          workbookName,
          entry,
          'vertical',
          startRow,
          endRow,
          address,
          boundaries,
          boundaries.length ? boundaries[boundaries.length - 1] : 0
        );

        if (!result.skipped) submitted += 1;
      }
    }

    if (submitted === 0) return;

    const results = await waitForProbeResults(workbookName, submitted);

    for (const result of results) {
      const entry = entries.find((candidate) => candidate.sheet === result.sheet);
      if (!entry) continue;

      const extent = Number(result.extent_units || 0);

      if (result.axis === 'horizontal') {
        entry.rightColumn = Math.min(
          MAX_COLUMNS - 1,
          Math.max(entry.rightColumn, Number(result.start_index) + extent - 1)
        );
      } else if (result.axis === 'vertical') {
        entry.bottomRow = Math.min(
          MAX_ROWS - 1,
          Math.max(entry.bottomRow, Number(result.start_index) + extent - 1)
        );
      }
    }
  }

  async function renderAddressBatch(workbookName, items, batchIndex, batchCount) {
    ExperimentUi.startBatch(
      batchIndex,
      batchCount,
      items.map((item) => item.sheet)
    );

    const rendered = await Excel.run(async (context) => {
      const workbook = context.workbook;
      const requests = [];
      const started = performance.now();

      for (const item of items) {
        const range = workbook.worksheets
          .getItem(item.sheet)
          .getRange(item.address);
        const image = range.getImage();
        requests.push({ item, image });
      }

      await context.sync();

      return {
        render_ms: performance.now() - started,
        images: requests.map((request) => ({
          id: request.item.id,
          sheet: request.item.sheet,
          address: request.item.address,
          base64: request.image.value
        }))
      };
    });

    const persistStarted = performance.now();
    const response = await fetch('/image-batch', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        workbook: workbookName,
        batch_index: batchIndex,
        images: rendered.images
      })
    });

    if (!response.ok) throw new Error(await response.text());
    const persisted = await response.json();
    const persistMs = performance.now() - persistStarted;

    ExperimentUi.completeBatch(
      batchIndex,
      batchCount,
      items.length,
      rendered.render_ms,
      persistMs
    );

    await ExperimentEvents.post('batch_completed', {
      workbook: workbookName,
      batch_index: batchIndex,
      batch_count: batchCount,
      image_count: items.length,
      sheets: items.map((item) => item.sheet),
      render_ms: fixed(rendered.render_ms),
      persist_ms: fixed(persistMs)
    });

    return {
      render_ms: rendered.render_ms,
      persist_ms: persistMs,
      captures: persisted.captures || []
    };
  }

  async function execute() {
    const configResponse = await fetch('/config');
    if (!configResponse.ok) throw new Error(await configResponse.text());
    const config = await configResponse.json();

    const workbookInfo = await getWorkbookAndVisibleSheets();
    const workbookName = workbookInfo.workbookName;
    const sheetNames = workbookInfo.sheets.map((sheet) => sheet.name);

    ExperimentUi.setWorkbook(workbookName);
    ExperimentUi.setPhase(`Planning strategy ${config.strategy}`);
    ExperimentUi.setTotalImages(sheetNames.length);

    await ExperimentEvents.post('measurement_started', {
      workbook: workbookName,
      strategy: config.strategy,
      visible_sheets: sheetNames.length
    });

    const planning = {
      used_range_ms: 0,
      content_shape_ms: 0,
      point_to_grid_ms: 0,
      overflow_probe_ms: 0
    };

    let plans = [];

    if (config.strategy === 'A') {
      const used = await resolveUsedRanges(sheetNames, false);
      planning.used_range_ms += used.elapsed_ms;
      plans = used.entries.map((entry, index) => ({
        id: `c${String(index + 1).padStart(5, '0')}`,
        sheet: entry.sheet,
        address: entry.address
      }));
    } else if (config.strategy === 'B') {
      // Strategy B resolves UsedRange immediately before each image batch below.
      plans = sheetNames.map((sheet, index) => ({
        id: `c${String(index + 1).padStart(5, '0')}`,
        sheet,
        address: null
      }));
    } else {
      const bounds = await collectContentAndShapeBounds(sheetNames);
      planning.content_shape_ms += bounds.elapsed_ms;
      planning.point_to_grid_ms += await mapPointBoundsToGrid(bounds.entries);

      if (config.strategy === 'C') {
        for (const entry of bounds.entries) {
          entry.rightColumn = Math.min(MAX_COLUMNS - 1, entry.rightColumn + 10);
          entry.bottomRow = Math.min(MAX_ROWS - 1, entry.bottomRow + 10);
        }
      }

      if (config.strategy === 'D' || config.strategy === 'E') {
        const started = performance.now();
        await runOverflowPlanning(workbookName, bounds.entries, config);
        planning.overflow_probe_ms += performance.now() - started;
      }

      plans = bounds.entries.map((entry, index) => ({
        id: `c${String(index + 1).padStart(5, '0')}`,
        sheet: entry.sheet,
        address: addressA1To(entry.bottomRow, entry.rightColumn),
        content_right_column: entry.contentRightColumn,
        content_bottom_row: entry.contentBottomRow,
        drawing_right_points: fixed(entry.shapeRightPoints),
        drawing_bottom_points: fixed(entry.shapeBottomPoints),
        visible_shape_count: entry.visibleShapeCount
      }));
    }

    const batchSize = Math.max(1, Number(config.batch_size || 8));
    const batchCount = Math.ceil(plans.length / batchSize);
    let renderTotal = 0;
    let persistTotal = 0;
    const persistedCaptures = [];
    const batches = [];

    for (let offset = 0; offset < plans.length; offset += batchSize) {
      const batchIndex = Math.floor(offset / batchSize) + 1;
      const batch = plans.slice(offset, offset + batchSize);

      if (config.strategy === 'B') {
        ExperimentUi.setPhase(
          `Resolving UsedRange for batch ${batchIndex} / ${batchCount}`
        );
        const used = await resolveUsedRanges(
          batch.map((item) => item.sheet),
          false
        );
        planning.used_range_ms += used.elapsed_ms;
        const bySheet = new Map(used.entries.map((entry) => [entry.sheet, entry]));
        for (const item of batch) {
          item.address = bySheet.get(item.sheet).address;
        }
      }

      const result = await renderAddressBatch(
        workbookName,
        batch,
        batchIndex,
        batchCount
      );

      renderTotal += result.render_ms;
      persistTotal += result.persist_ms;
      persistedCaptures.push(...result.captures);

      const total = result.render_ms + result.persist_ms;
      batches.push({
        batch_index: batchIndex,
        capture_count: batch.length,
        sheets: batch.map((item) => item.sheet),
        render_ms: fixed(result.render_ms),
        persist_ms: fixed(result.persist_ms),
        total_ms: fixed(total),
        amortized_ms_per_image: fixed(total / Math.max(1, batch.length))
      });
    }

    const payload = {
      workbook: workbookName,
      strategy: config.strategy,
      planning: Object.fromEntries(
        Object.entries(planning).map(([key, value]) => [key, fixed(value)])
      ),
      render_total_ms: fixed(renderTotal),
      persist_total_ms: fixed(persistTotal),
      capture_total_ms: fixed(renderTotal + persistTotal),
      image_batch_size: batchSize,
      capture_count: persistedCaptures.length,
      batches,
      captures: persistedCaptures,
      excel_api_requirement_set: '1.11'
    };

    const response = await fetch('/result', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload)
    });
    if (!response.ok) throw new Error(await response.text());

    await ExperimentEvents.post('result_submitted', {
      workbook: workbookName,
      strategy: config.strategy,
      capture_count: payload.capture_count,
      capture_total_ms: payload.capture_total_ms
    });

    ExperimentUi.setPhase('Complete · waiting for host to close workbook');
  }

  try {
    await execute();
  } catch (error) {
    const detail = error && error.stack ? error.stack : String(error);
    ExperimentUi.fail(detail);
    await ExperimentEvents.post('error', { message: detail });
    try {
      await fetch('/error', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: detail })
      });
    } catch (_) {
    }
  }
}());
