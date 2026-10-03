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

  async function collectTrailingHiddenBounds(sheetNames) {
    const states = sheetNames.map((sheet) => ({
      sheet,
      column: { terminalHidden: false, low: 0, high: MAX_COLUMNS - 1 },
      row: { terminalHidden: false, low: 0, high: MAX_ROWS - 1 }
    }));
    const started = performance.now();

    await Excel.run(async (context) => {
      const workbook = context.workbook;
      const probes = [];

      for (const state of states) {
        const sheet = workbook.worksheets.getItem(state.sheet);
        const lastColumn = sheet.getRangeByIndexes(0, MAX_COLUMNS - 1, 1, 1);
        const lastRow = sheet.getRangeByIndexes(MAX_ROWS - 1, 0, 1, 1);
        lastColumn.load('columnHidden');
        lastRow.load('rowHidden');
        probes.push({ state, lastColumn, lastRow });
      }

      await context.sync();

      for (const probe of probes) {
        probe.state.column.terminalHidden = probe.lastColumn.columnHidden === true;
        probe.state.row.terminalHidden = probe.lastRow.rowHidden === true;
      }
    });

    while (states.some((state) =>
      (state.column.terminalHidden && state.column.low < state.column.high) ||
      (state.row.terminalHidden && state.row.low < state.row.high)
    )) {
      await Excel.run(async (context) => {
        const workbook = context.workbook;
        const probes = [];

        for (const state of states) {
          const sheet = workbook.worksheets.getItem(state.sheet);

          if (state.column.terminalHidden && state.column.low < state.column.high) {
            const mid = Math.floor((state.column.low + state.column.high) / 2);
            const range = sheet.getRangeByIndexes(
              0,
              mid,
              1,
              MAX_COLUMNS - mid
            );
            range.load('columnHidden');
            probes.push({ axis: 'column', state, mid, range });
          }

          if (state.row.terminalHidden && state.row.low < state.row.high) {
            const mid = Math.floor((state.row.low + state.row.high) / 2);
            const range = sheet.getRangeByIndexes(
              mid,
              0,
              MAX_ROWS - mid,
              1
            );
            range.load('rowHidden');
            probes.push({ axis: 'row', state, mid, range });
          }
        }

        await context.sync();

        for (const probe of probes) {
          const entirelyHidden = probe.axis === 'column'
            ? probe.range.columnHidden === true
            : probe.range.rowHidden === true;

          if (entirelyHidden) {
            if (probe.axis === 'column') probe.state.column.high = probe.mid;
            else probe.state.row.high = probe.mid;
          } else {
            if (probe.axis === 'column') probe.state.column.low = probe.mid + 1;
            else probe.state.row.low = probe.mid + 1;
          }
        }
      });
    }

    return {
      elapsed_ms: performance.now() - started,
      entries: states.map((state) => ({
        sheet: state.sheet,
        trailingHiddenColumnStart: state.column.terminalHidden
          ? state.column.low
          : null,
        trailingHiddenRowStart: state.row.terminalHidden
          ? state.row.low
          : null,
        rightColumn: state.column.terminalHidden
          ? Math.max(0, state.column.low - 1)
          : null,
        bottomRow: state.row.terminalHidden
          ? Math.max(0, state.row.low - 1)
          : null
      }))
    };
  }

  async function collectContentAndShapeBounds(sheetNames, trailingBounds) {
    const trailingBySheet = new Map(
      trailingBounds.map((entry) => [entry.sheet, entry])
    );

    return Excel.run(async (context) => {
      const workbook = context.workbook;
      const entries = [];

      for (const sheetName of sheetNames) {
        const sheet = workbook.worksheets.getItem(sheetName);
        const trailing = trailingBySheet.get(sheetName) || {};
        const horizontalFixed = Number.isInteger(trailing.rightColumn);
        const verticalFixed = Number.isInteger(trailing.bottomRow);
        let content = null;
        let shapes = null;

        if (!horizontalFixed || !verticalFixed) {
          content = sheet.getUsedRangeOrNullObject(true);
          content.load('address,rowIndex,columnIndex,rowCount,columnCount,left,top,width,height');

          shapes = sheet.shapes;
          shapes.load(
            'items/left,items/top,items/width,items/height,' +
            'items/rotation,items/visible'
          );
        }

        entries.push({
          sheetName,
          trailing,
          horizontalFixed,
          verticalFixed,
          content,
          shapes
        });
      }

      const started = performance.now();
      await context.sync();
      const elapsed = performance.now() - started;

      return {
        elapsed_ms: elapsed,
        entries: entries.map((entry) => {
          const content = entry.content === null || entry.content.isNullObject
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

          if (entry.shapes !== null) {
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

              if (!entry.horizontalFixed) {
                shapeRightPoints = Math.max(
                  shapeRightPoints,
                  Number(shape.left) + width / 2 + rotatedWidth / 2
                );
              }
              if (!entry.verticalFixed) {
                shapeBottomPoints = Math.max(
                  shapeBottomPoints,
                  Number(shape.top) + height / 2 + rotatedHeight / 2
                );
              }
            }
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
            trailingHiddenColumnStart: entry.trailing.trailingHiddenColumnStart ?? null,
            trailingHiddenRowStart: entry.trailing.trailingHiddenRowStart ?? null,
            rightBoundSource: entry.horizontalFixed ? 'trailing-hidden' : 'content-drawing',
            bottomBoundSource: entry.verticalFixed ? 'trailing-hidden' : 'content-drawing',
            rightColumn: entry.horizontalFixed
              ? Number(entry.trailing.rightColumn)
              : contentRightColumn,
            bottomRow: entry.verticalFixed
              ? Number(entry.trailing.bottomRow)
              : contentBottomRow,
            overflowProbes: []
          };
        })
      };
    });
  }

  async function mapPointBoundsToGrid(entries) {
    const horizontal = entries
      .filter((entry) =>
        entry.rightBoundSource !== 'trailing-hidden' &&
        entry.shapeRightPoints > entry.contentRightPoints + 0.01
      )
      .map((entry) => ({
        entry,
        low: Math.max(0, entry.contentRightColumn),
        high: MAX_COLUMNS - 1,
        target: entry.shapeRightPoints
      }));

    const vertical = entries
      .filter((entry) =>
        entry.bottomBoundSource !== 'trailing-hidden' &&
        entry.shapeBottomPoints > entry.contentBottomPoints + 0.01
      )
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

    async function applyVisibleStrategyCMargin(entries, marginCount) {
    return Excel.run(async (context) => {
      const workbook = context.workbook;
      const checks = [];

      for (const entry of entries) {
        const sheet = workbook.worksheets.getItem(entry.sheet);
        const columns = [];
        const rows = [];

        if (entry.rightBoundSource !== 'trailing-hidden') {
          for (let offset = 1; offset <= marginCount; offset += 1) {
            const columnIndex = entry.rightColumn + offset;
            if (columnIndex >= MAX_COLUMNS) break;

            const cell = sheet.getRangeByIndexes(0, columnIndex, 1, 1);
            cell.load('columnHidden');
            columns.push({ index: columnIndex, cell });
          }
        }

        if (entry.bottomBoundSource !== 'trailing-hidden') {
          for (let offset = 1; offset <= marginCount; offset += 1) {
            const rowIndex = entry.bottomRow + offset;
            if (rowIndex >= MAX_ROWS) break;

            const cell = sheet.getRangeByIndexes(rowIndex, 0, 1, 1);
            cell.load('rowHidden');
            rows.push({ index: rowIndex, cell });
          }
        }

        checks.push({ entry, columns, rows });
      }

      await context.sync();

      for (const check of checks) {
        for (const column of check.columns) {
          if (column.cell.columnHidden === true) break;
          check.entry.rightColumn = column.index;
        }

        for (const row of check.rows) {
          if (row.cell.rowHidden === true) break;
          check.entry.bottomRow = row.index;
        }
      }
    });
  }

async function readColumnHidden(sheetName, startColumn, endColumn) {
    if (endColumn < startColumn) return true;

    return Excel.run(async (context) => {
      const range = context.workbook.worksheets
        .getItem(sheetName)
        .getRangeByIndexes(
          0,
          startColumn,
          1,
          endColumn - startColumn + 1
        );
      range.load('columnHidden');
      await context.sync();
      return range.columnHidden === true;
    });
  }

  async function readRowHidden(sheetName, startRow, endRow) {
    if (endRow < startRow) return true;

    return Excel.run(async (context) => {
      const range = context.workbook.worksheets
        .getItem(sheetName)
        .getRangeByIndexes(
          startRow,
          0,
          endRow - startRow + 1,
          1
        );
      range.load('rowHidden');
      await context.sync();
      return range.rowHidden === true;
    });
  }

  async function findFirstVisibleProbeColumn(sheetName, startColumn) {
    if (startColumn >= MAX_COLUMNS) return -1;

    if (!(await readColumnHidden(sheetName, startColumn, startColumn))) {
      return startColumn;
    }

    if (await readColumnHidden(sheetName, startColumn, MAX_COLUMNS - 1)) {
      return -1;
    }

    let low = startColumn + 1;
    let high = MAX_COLUMNS - 1;

    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      if (await readColumnHidden(sheetName, startColumn, mid)) {
        low = mid + 1;
      } else {
        high = mid;
      }
    }

    return low;
  }

  async function findLastVisibleProbeColumn(sheetName, startColumn) {
    if (startColumn >= MAX_COLUMNS) return -1;
    if (await readColumnHidden(sheetName, startColumn, MAX_COLUMNS - 1)) {
      return -1;
    }

    let low = startColumn;
    let high = MAX_COLUMNS - 1;

    while (low < high) {
      const mid = Math.ceil((low + high) / 2);
      if (await readColumnHidden(sheetName, mid, MAX_COLUMNS - 1)) {
        high = mid - 1;
      } else {
        low = mid;
      }
    }

    return low;
  }

  async function findFirstVisibleProbeRow(sheetName, startRow) {
    if (startRow >= MAX_ROWS) return -1;

    if (!(await readRowHidden(sheetName, startRow, startRow))) {
      return startRow;
    }

    if (await readRowHidden(sheetName, startRow, MAX_ROWS - 1)) {
      return -1;
    }

    let low = startRow + 1;
    let high = MAX_ROWS - 1;

    while (low < high) {
      const mid = Math.floor((low + high) / 2);
      if (await readRowHidden(sheetName, startRow, mid)) {
        low = mid + 1;
      } else {
        high = mid;
      }
    }

    return low;
  }

  async function findLastVisibleProbeRow(sheetName, startRow) {
    if (startRow >= MAX_ROWS) return -1;
    if (await readRowHidden(sheetName, startRow, MAX_ROWS - 1)) {
      return -1;
    }

    let low = startRow;
    let high = MAX_ROWS - 1;

    while (low < high) {
      const mid = Math.ceil((low + high) / 2);
      if (await readRowHidden(sheetName, mid, MAX_ROWS - 1)) {
        high = mid - 1;
      } else {
        low = mid;
      }
    }

    return low;
  }

  async function probeEndColumnByPixelWidth(sheetName, startColumn, targetPoints) {
    const totalVisibleWidth = await Excel.run(async (context) => {
      const range = context.workbook.worksheets
        .getItem(sheetName)
        .getRangeByIndexes(
          0,
          startColumn,
          1,
          MAX_COLUMNS - startColumn
        );
      range.load('width');
      await context.sync();
      return Number(range.width);
    });

    let low = startColumn;
    let high = MAX_COLUMNS - 1;

    // If the remaining visible width is smaller than the requested probe width,
    // stop at the last visible column instead of extending through a trailing
    // run of hidden columns all the way to XFD.
    if (totalVisibleWidth < targetPoints) {
      const lastVisible = await findLastVisibleProbeColumn(
        sheetName,
        startColumn
      );
      return lastVisible >= 0 ? lastVisible : startColumn;
    }

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
    const totalVisibleHeight = await Excel.run(async (context) => {
      const range = context.workbook.worksheets
        .getItem(sheetName)
        .getRangeByIndexes(
          startRow,
          0,
          MAX_ROWS - startRow,
          1
        );
      range.load('height');
      await context.sync();
      return Number(range.height);
    });

    let low = startRow;
    let high = MAX_ROWS - 1;

    if (totalVisibleHeight < targetPoints) {
      const lastVisible = await findLastVisibleProbeRow(sheetName, startRow);
      return lastVisible >= 0 ? lastVisible : startRow;
    }

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

  function captureColumnsToWidths(columns) {
    const defaultWidth = Number(columns && columns.default_width);
    const fallback = Number.isFinite(defaultWidth) && defaultWidth >= 0
      ? defaultWidth
      : 8.43;
    const widths = new Array(MAX_COLUMNS).fill(fallback);
    for (const span of (columns && columns.spans) || []) {
      const first = Math.max(0, Number(span.min) - 1);
      const last = Math.min(MAX_COLUMNS - 1, Number(span.max) - 1);
      const explicit = span.width === null || span.width === undefined
        ? fallback
        : Number(span.width);
      const width = span.hidden ? 0 : explicit;
      if (!Number.isFinite(width) || width < 0 || first > last) {
        throw new Error('Invalid precomputed column plan.');
      }
      for (let column = first; column <= last; column += 1) {
        widths[column] = width;
      }
    }
    return widths;
  }

  function probeEndByVisibleColumns(widths, baseColumn, visibleCount) {
    let found = 0;
    for (let column = Number(baseColumn) + 1; column < MAX_COLUMNS; column += 1) {
      if (Number(widths[column]) > 0) {
        found += 1;
        if (found >= visibleCount) return column;
      }
    }
    return found === 0 ? Number(baseColumn) : MAX_COLUMNS - 1;
  }

  async function fetchPrecomputedVisualPlan(workbookName) {
    const started = performance.now();
    const response = await fetch('/visual-plan.json', { cache: 'no-store' });
    if (!response.ok) throw new Error(await response.text());
    const document = await response.json();
    const plan = (document.workbooks || []).find(
      (candidate) => String(candidate.workbook || '').toLowerCase() ===
        String(workbookName || '').toLowerCase()
    );
    if (!plan) {
      throw new Error(
        `Precomputed WSL Rust visual plan has no workbook named ${workbookName}.`
      );
    }
    return {
      plan,
      fetch_ms: performance.now() - started,
      total_rust_plan_ms: Number(document.total_rust_plan_ms || 0),
      schema: document.schema || null
    };
  }

  function entriesFromPrecomputedPlan(sheetNames, plan) {
    const byName = new Map((plan.sheets || []).map((sheet) => [sheet.sheet, sheet]));
    return sheetNames.map((sheetName) => {
      const source = byName.get(sheetName);
      if (!source) {
        throw new Error(
          `Precomputed WSL Rust visual plan has no visible worksheet named ${sheetName}.`
        );
      }
      return {
        sheet: sheetName,
        contentRightColumn: Number(source.content_right_column || 0),
        contentBottomRow: Number(source.content_bottom_row || 0),
        contentRightPoints: 0,
        contentBottomPoints: 0,
        shapeRightPoints: 0,
        shapeBottomPoints: 0,
        visibleShapeCount: 0,
        drawingRightColumn: Number(source.drawing_right_column || 0),
        drawingBottomRow: Number(source.drawing_bottom_row || 0),
        drawingFallbackRequired: Boolean(source.drawing_fallback_required),
        drawingAnchorCount: Number(source.drawing_anchor_count || 0),
        trailingHiddenColumnStart: source.trailing_hidden_column_start ?? null,
        trailingHiddenRowStart: source.trailing_hidden_row_start ?? null,
        rightBoundSource: source.right_bound_source || 'wsl-rust-xml-content-drawing',
        bottomBoundSource: source.bottom_bound_source || 'wsl-rust-xml-content-drawing',
        rightColumn: Number(source.base_right_column || 0),
        bottomRow: Number(source.bottom_row || 0),
        hasText: Boolean(source.has_text),
        horizontalOverflowRequired: Boolean(source.horizontal_overflow_required),
        columns: source.columns || { default_width: 8.43, spans: [] },
        columnWidths: captureColumnsToWidths(source.columns || {}),
        overflowProbes: []
      };
    });
  }

  async function collectFallbackShapeBounds(entries) {
    const targets = entries.filter((entry) =>
      entry.drawingFallbackRequired &&
      (entry.rightBoundSource !== 'trailing-hidden' ||
       entry.bottomBoundSource !== 'trailing-hidden')
    );
    if (targets.length === 0) return 0;

    const started = performance.now();
    await Excel.run(async (context) => {
      const workbook = context.workbook;
      const pending = [];
      for (const entry of targets) {
        const shapes = workbook.worksheets.getItem(entry.sheet).shapes;
        shapes.load(
          'items/left,items/top,items/width,items/height,' +
          'items/rotation,items/visible'
        );
        pending.push({ entry, shapes });
      }
      await context.sync();

      for (const item of pending) {
        let right = 0;
        let bottom = 0;
        let count = 0;
        for (const shape of item.shapes.items) {
          if (!shape.visible) continue;
          count += 1;
          const angle = Number(shape.rotation) * Math.PI / 180.0;
          const width = Number(shape.width);
          const height = Number(shape.height);
          const rotatedWidth =
            Math.abs(width * Math.cos(angle)) + Math.abs(height * Math.sin(angle));
          const rotatedHeight =
            Math.abs(width * Math.sin(angle)) + Math.abs(height * Math.cos(angle));
          right = Math.max(
            right,
            Number(shape.left) + width / 2 + rotatedWidth / 2
          );
          bottom = Math.max(
            bottom,
            Number(shape.top) + height / 2 + rotatedHeight / 2
          );
        }
        item.entry.shapeRightPoints = right;
        item.entry.shapeBottomPoints = bottom;
        item.entry.visibleShapeCount = count;
      }
    });
    return performance.now() - started;
  }

  function precomputedPlannedItems(entries) {
    return entries.map((entry, index) => {
      const initialExtra = 8;
      const requestedEnd = probeEndByVisibleColumns(
        entry.columnWidths,
        entry.rightColumn,
        initialExtra
      );
      const overflowProbe =
        entry.rightBoundSource !== 'trailing-hidden' &&
        entry.hasText &&
        entry.horizontalOverflowRequired &&
        requestedEnd > entry.rightColumn;
      const endColumn = overflowProbe ? requestedEnd : entry.rightColumn;
      return {
        id: `c${String(index + 1).padStart(5, '0')}`,
        sheet: entry.sheet,
        address: addressA1To(entry.bottomRow, endColumn),
        base_right_column: entry.rightColumn,
        bottom_row: entry.bottomRow,
        end_column: endColumn,
        overflow_probe: overflowProbe,
        probe_extra: overflowProbe ? initialExtra : 0,
        content_right_column: entry.contentRightColumn,
        content_bottom_row: entry.contentBottomRow,
        drawing_right_column: entry.drawingRightColumn,
        drawing_bottom_row: entry.drawingBottomRow,
        drawing_fallback_required: entry.drawingFallbackRequired,
        visible_shape_count: entry.visibleShapeCount,
        right_bound_source: entry.rightBoundSource,
        bottom_bound_source: entry.bottomBoundSource,
        trailing_hidden_column_start: entry.trailingHiddenColumnStart,
        trailing_hidden_row_start: entry.trailingHiddenRowStart,
        column_widths: entry.columnWidths,
        overflow_passes: []
      };
    });
  }

  function columnEdgesFromWidths(widths, endColumn, imageWidth) {
    if (
      !Number.isInteger(endColumn) ||
      endColumn < 0 ||
      endColumn >= MAX_COLUMNS ||
      imageWidth <= 0
    ) {
      throw new Error('Invalid render-overflow image extent.');
    }

    let total = 0;
    for (let column = 0; column <= endColumn; column += 1) {
      total += Number(widths[column] || 0);
    }
    if (!Number.isFinite(total) || total <= 0) {
      throw new Error('Render-overflow capture has no visible XML column width.');
    }

    const scaleWidth = Math.max(1, imageWidth - 1);
    const edges = [0];
    let cumulative = 0;
    for (let column = 0; column <= endColumn; column += 1) {
      cumulative += Number(widths[column] || 0);
      edges.push(Math.round(cumulative / total * scaleWidth));
    }
    edges[edges.length - 1] = scaleWidth;
    return edges;
  }

  function pixelOffset(image, x, y) {
    return (y * image.width + x) * 4;
  }

  function pixelDiffers(image, offset, background) {
    const data = image.data;
    return data[offset + 3] > 0 &&
      (
        Math.abs(data[offset] - background[0]) > 14 ||
        Math.abs(data[offset + 1] - background[1]) > 14 ||
        Math.abs(data[offset + 2] - background[2]) > 14
      );
  }

  function estimateRenderBackground(image, edges, baseColumn, endColumn) {
    const startColumn = Math.min(baseColumn + 1, endColumn);
    const x0 = Math.min(
      image.width - 1,
      Math.max(0, Number(edges[startColumn] || 0))
    );
    const x1 = Math.min(
      image.width,
      Math.max(x0 + 1, Number(edges[endColumn + 1] || image.width))
    );
    const area = Math.max(1, (x1 - x0) * image.height);
    const stride = Math.max(1, Math.floor(Math.sqrt(Math.max(1, area / 100000))));
    const histogram = new Map();

    for (let y = 0; y < image.height; y += stride) {
      for (let x = x0; x < x1; x += stride) {
        const offset = pixelOffset(image, x, y);
        const key =
          ((image.data[offset] >> 4) << 8) |
          ((image.data[offset + 1] >> 4) << 4) |
          (image.data[offset + 2] >> 4);
        histogram.set(key, Number(histogram.get(key) || 0) + 1);
      }
    }

    let bestKey = 0x0fff;
    let bestCount = -1;
    for (const [key, count] of histogram.entries()) {
      if (count > bestCount) {
        bestKey = key;
        bestCount = count;
      }
    }
    return [
      ((bestKey >> 8) & 0x0f) * 16 + 8,
      ((bestKey >> 4) & 0x0f) * 16 + 8,
      (bestKey & 0x0f) * 16 + 8,
      255
    ];
  }

  function horizontalRuleMask(image, background) {
    const candidates = new Uint8Array(image.height);
    if (image.width < 16 || image.height < 3) return candidates;

    for (let y = 1; y < image.height - 1; y += 1) {
      const referenceOffset = pixelOffset(image, 1, y);
      if (!pixelDiffers(image, referenceOffset, background)) continue;

      const r = image.data[referenceOffset];
      const g = image.data[referenceOffset + 1];
      const b = image.data[referenceOffset + 2];
      let uniform = true;

      for (let x = 2; x < image.width - 1; x += 1) {
        const offset = pixelOffset(image, x, y);
        if (
          Math.abs(image.data[offset] - r) > 8 ||
          Math.abs(image.data[offset + 1] - g) > 8 ||
          Math.abs(image.data[offset + 2] - b) > 8
        ) {
          uniform = false;
          break;
        }
      }
      if (uniform) candidates[y] = 1;
    }

    const mask = new Uint8Array(image.height);
    let start = 1;
    while (start < candidates.length) {
      if (!candidates[start]) {
        start += 1;
        continue;
      }
      let end = start + 1;
      while (end < candidates.length && candidates[end]) end += 1;
      if (end - start <= 4) mask.fill(1, start, end);
      start = end;
    }
    return mask;
  }

  function analyzeRenderOverflow(image, widths, baseColumn, endColumn, extra) {
    const edges = columnEdgesFromWidths(widths, endColumn, image.width);
    const background = estimateRenderBackground(
      image,
      edges,
      baseColumn,
      endColumn
    );
    const rules = horizontalRuleMask(image, background);

    let lastVisible = -1;
    for (let column = baseColumn + 1; column <= endColumn; column += 1) {
      if (Number(widths[column]) > 0) lastVisible = column;
    }

    let moreVisible = false;
    for (let column = endColumn + 1; column < MAX_COLUMNS; column += 1) {
      if (Number(widths[column]) > 0) {
        moreVisible = true;
        break;
      }
    }

    let keepColumn = baseColumn;
    let edgeContent = false;
    let scannedPixels = 0;
    let ignoredRuleRows = 0;
    for (let y = 0; y < rules.length; y += 1) {
      if (rules[y]) ignoredRuleRows += 1;
    }

    for (let column = baseColumn + 1; column <= endColumn; column += 1) {
      if (Number(widths[column]) <= 0) continue;
      const left = Math.max(0, Number(edges[column] || 0));
      const right = Math.min(image.width, Number(edges[column + 1] || image.width));
      if (right - left <= 2) continue;

      let content = false;
      scanColumn:
      for (let y = 1; y < image.height - 1; y += 1) {
        if (rules[y]) continue;
        for (let x = left + 1; x < right - 1; x += 1) {
          scannedPixels += 1;
          if (pixelDiffers(image, pixelOffset(image, x, y), background)) {
            content = true;
            break scanColumn;
          }
        }
      }

      if (content) {
        keepColumn = Math.max(keepColumn, column);
        if (column === lastVisible) edgeContent = true;
      }
    }

    const nextExtra = extra < 8
      ? 8
      : Math.min(MAX_COLUMNS, Math.max(8, extra * 2));
    const nextColumn = probeEndByVisibleColumns(widths, baseColumn, nextExtra);
    const done = !edgeContent || !moreVisible || nextColumn <= endColumn;

    return {
      done,
      keep_column: keepColumn,
      next_column: nextColumn,
      next_extra: nextExtra,
      scanned_pixels: scannedPixels,
      ignored_rule_rows: ignoredRuleRows,
      background_rgba: background,
      edges
    };
  }

  async function decodePngForOverflow(base64) {
    const imageElement = await new Promise((resolve, reject) => {
      const image = new Image();
      image.onload = () => resolve(image);
      image.onerror = () => reject(new Error('Could not decode Range.getImage() PNG.'));
      image.src = `data:image/png;base64,${base64}`;
    });

    const width = Number(imageElement.naturalWidth || imageElement.width);
    const height = Number(imageElement.naturalHeight || imageElement.height);
    if (!(width > 0 && height > 0)) {
      throw new Error('Decoded Range.getImage() PNG has invalid dimensions.');
    }

    const canvas = document.createElement('canvas');
    canvas.width = width;
    canvas.height = height;
    const context = canvas.getContext('2d', { willReadFrequently: true });
    if (!context) throw new Error('Canvas 2D context is unavailable.');
    context.drawImage(imageElement, 0, 0);
    const imageData = context.getImageData(0, 0, width, height);

    return {
      width,
      height,
      data: imageData.data,
      canvas
    };
  }

  function cropOverflowBase64(image, edges, keepColumn, originalBase64) {
    const rightEdge = Number(edges[keepColumn + 1] || image.width - 1);
    const cropWidth = Math.max(
      1,
      Math.min(image.width, Math.round(rightEdge) + 1)
    );
    if (cropWidth >= image.width) return originalBase64;

    const canvas = document.createElement('canvas');
    canvas.width = cropWidth;
    canvas.height = image.height;
    const context = canvas.getContext('2d');
    if (!context) throw new Error('Canvas 2D context is unavailable for crop.');
    context.drawImage(
      image.canvas,
      0,
      0,
      cropWidth,
      image.height,
      0,
      0,
      cropWidth,
      image.height
    );
    const dataUrl = canvas.toDataURL('image/png');
    const comma = dataUrl.indexOf(',');
    if (comma < 0) throw new Error('Canvas PNG encoding did not return a data URL.');
    return dataUrl.slice(comma + 1);
  }

  async function renderPrecomputedPlannedBatch(
    workbookName,
    items,
    batchIndex,
    batchCount
  ) {
    ExperimentUi.startBatch(
      batchIndex,
      batchCount,
      items.map((item) => item.sheet)
    );

    const planById = new Map(items.map((item) => [item.id, item]));
    let pending = items.map((item) => ({ ...item }));
    const captures = [];
    let renderMs = 0;
    let persistMs = 0;
    let analysisMs = 0;
    let iteration = 0;

    while (pending.length > 0) {
      iteration += 1;
      const rendered = await Excel.run(async (context) => {
        const workbook = context.workbook;
        const requests = [];
        const started = performance.now();
        for (const item of pending) {
          const address = addressA1To(item.bottom_row, item.end_column);
          const image = workbook.worksheets
            .getItem(item.sheet)
            .getRange(address)
            .getImage();
          requests.push({ item, address, image });
        }

        // Source Context patch: diagnostic breakpoints around Strategy F getImage sync.
        // Pause before Excel executes the batched Range.getImage() requests.
        debugger;
        try {
          await context.sync();
        } catch (error) {
          // Pause again with the actual RichApi error and request list in scope.
          debugger;
          const detail = officeErrorDetail(error);
          throw new Error(
            `Strategy F Range.getImage context.sync failed: ${detail}`
          );
        }
        return {
          render_ms: performance.now() - started,
          images: requests.map((request) => ({
            item: request.item,
            address: request.address,
            base64: request.image.value
          }))
        };
      });
      renderMs += rendered.render_ms;

      const finalized = [];
      const nextPending = [];
      const analysisStarted = performance.now();

      for (const renderedImage of rendered.images) {
        const item = renderedImage.item;
        const canonical = planById.get(item.id);
        if (!canonical) {
          throw new Error(`Missing Strategy F capture plan for ${item.id}.`);
        }

        if (!item.overflow_probe) {
          canonical.address = renderedImage.address;
          canonical.end_column = item.end_column;
          canonical.overflow_checked = false;
          finalized.push({
            id: item.id,
            sheet: item.sheet,
            address: renderedImage.address,
            base64: renderedImage.base64
          });
          continue;
        }

        const decoded = await decodePngForOverflow(renderedImage.base64);
        const decision = analyzeRenderOverflow(
          decoded,
          item.column_widths,
          item.base_right_column,
          item.end_column,
          item.probe_extra
        );
        const pass = {
          capture_end_column: item.end_column,
          probe_extra: item.probe_extra,
          keep_column: decision.keep_column,
          edge_reached: !decision.done,
          scanned_pixels: decision.scanned_pixels,
          ignored_rule_rows: decision.ignored_rule_rows,
          background_rgba: decision.background_rgba
        };
        const passes = [...(item.overflow_passes || []), pass];

        if (!decision.done) {
          if (
            !Number.isInteger(decision.next_column) ||
            decision.next_column <= item.end_column
          ) {
            throw new Error(
              `Browser overflow planning made no progress for ${item.sheet}.`
            );
          }
          canonical.end_column = decision.next_column;
          canonical.probe_extra = decision.next_extra;
          canonical.overflow_passes = passes;
          nextPending.push({
            ...item,
            end_column: decision.next_column,
            probe_extra: decision.next_extra,
            overflow_passes: passes
          });
          continue;
        }

        const finalAddress = addressA1To(item.bottom_row, decision.keep_column);
        const finalBase64 = cropOverflowBase64(
          decoded,
          decision.edges,
          decision.keep_column,
          renderedImage.base64
        );
        canonical.address = finalAddress;
        canonical.end_column = decision.keep_column;
        canonical.overflow_checked = true;
        canonical.overflow_keep_column = decision.keep_column;
        canonical.overflow_passes = passes;
        canonical.overflow_scanned_pixels = passes.reduce(
          (sum, current) => sum + Number(current.scanned_pixels || 0),
          0
        );
        finalized.push({
          id: item.id,
          sheet: item.sheet,
          address: finalAddress,
          base64: finalBase64
        });
      }

      analysisMs += performance.now() - analysisStarted;

      if (finalized.length > 0) {
        const persistStarted = performance.now();
        const response = await fetch('/image-batch', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            workbook: workbookName,
            batch_index: batchIndex,
            images: finalized
          })
        });
        if (!response.ok) throw new Error(await response.text());
        const result = await response.json();
        persistMs += performance.now() - persistStarted;
        captures.push(...(result.captures || []));
      }

      pending = nextPending;
      ExperimentUi.showAnalysis(
        captures.length,
        items.length,
        pending.length === 0
          ? 'Precomputed WSL Rust bounds + browser overflow analysis complete.'
          : `Overflow pass ${iteration}: ${pending.length} sheet(s) need a wider image.`
      );
    }

    captures.sort((left, right) => String(left.id).localeCompare(String(right.id)));

    ExperimentUi.completeBatch(
      batchIndex,
      batchCount,
      items.length,
      renderMs,
      persistMs
    );
    await ExperimentEvents.post('batch_completed', {
      workbook: workbookName,
      batch_index: batchIndex,
      batch_count: batchCount,
      image_count: items.length,
      sheets: items.map((item) => item.sheet),
      render_ms: fixed(renderMs),
      persist_ms: fixed(persistMs),
      browser_overflow_analysis_ms: fixed(analysisMs)
    });
    return {
      render_ms: renderMs,
      persist_ms: persistMs,
      analysis_ms: analysisMs,
      captures
    };
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
    probeId,
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
      //   horizontal overflow -> first visible column to the right
      //   vertical overflow   -> first visible row below
      // The image still covers the complete probe range, so the measured XOR
      // extent is unchanged while the mutation is dramatically smaller.
      const blocker = axis === 'horizontal'
        ? sheet.getRangeByIndexes(
            0,
            startIndex,
            Math.max(1, Number(entry.bottomRow) + 1),
            1
          )
        : sheet.getRangeByIndexes(
            startIndex,
            0,
            1,
            Math.max(1, Number(entry.rightColumn) + 1)
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
          probe_id: probeId,
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

      if (
        horizontal &&
        entry.rightBoundSource !== 'trailing-hidden' &&
        entry.rightColumn < MAX_COLUMNS - 1
      ) {
        const requestedStartColumn = entry.rightColumn + 1;
        const startColumn = await findFirstVisibleProbeColumn(
          entry.sheet,
          requestedStartColumn
        );

        if (startColumn < 0) {
          ExperimentUi.log(
            `Skipping horizontal overflow probe for ${entry.sheet}: ` +
            'all remaining columns are hidden.'
          );
        } else {
          const targetPoints =
            Number(config.overflow_probe_width_px || 1920) * 72 / 96;
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
            `${columnName(endColumn)}${Math.max(1, entry.bottomRow + 1)}`;

          const result = await captureProbePair(
            workbookName,
            `h${String(index + 1).padStart(5, '0')}`,
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
      }

      if (
        vertical &&
        entry.bottomBoundSource !== 'trailing-hidden' &&
        entry.bottomRow < MAX_ROWS - 1
      ) {
        const requestedStartRow = entry.bottomRow + 1;
        const startRow = await findFirstVisibleProbeRow(
          entry.sheet,
          requestedStartRow
        );

        if (startRow < 0) {
          ExperimentUi.log(
            `Skipping vertical overflow probe for ${entry.sheet}: ` +
            'all remaining rows are hidden.'
          );
        } else {
          const targetPoints =
            Number(config.overflow_probe_height_px || 1080) * 72 / 96;
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
          const lastColumn = Math.max(0, entry.rightColumn);
          const address =
            `A${startRow + 1}:${columnName(lastColumn)}${endRow + 1}`;

          const result = await captureProbePair(
            workbookName,
            `v${String(index + 1).padStart(5, '0')}`,
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
    }

    if (submitted === 0) return;

    const results = await waitForProbeResults(workbookName, submitted);

    for (const result of results) {
      const entry = entries.find((candidate) => candidate.sheet === result.sheet);
      if (!entry) continue;

      const extent = Number(result.extent_units || 0);
      entry.overflowProbes.push({
        probe_id: result.probe_id || null,
        axis: result.axis,
        start_index: Number(result.start_index),
        extent_units: extent,
        changed_pixel: Number(result.changed_pixel),
        changed_points: Number(result.changed_points),
        before_png: result.before_png || null,
        after_png: result.after_png || null,
        xor_png: result.xor_png || null
      });

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
      trailing_hidden_bounds_ms: 0,
      content_shape_ms: 0,
      point_to_grid_ms: 0,
      overflow_probe_ms: 0,
      rust_xml_plan_ms: 0,
      rust_plan_fetch_ms: 0,
      fallback_shape_ms: 0
    };

    let plans = [];
    let rustVisualPlan = null;

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
    } else if (config.strategy === 'F') {
      ExperimentUi.setPhase('Loading precomputed WSL Rust OOXML visual plan');
      const loadedPlan = await fetchPrecomputedVisualPlan(workbookName);
      rustVisualPlan = loadedPlan.plan;
      planning.rust_xml_plan_ms += Number(rustVisualPlan.rust_plan_ms || 0);
      planning.rust_plan_fetch_ms += loadedPlan.fetch_ms;

      const entries = entriesFromPrecomputedPlan(sheetNames, rustVisualPlan);
      planning.fallback_shape_ms += await collectFallbackShapeBounds(entries);
      const fallbackEntries = entries.filter((entry) => entry.drawingFallbackRequired);
      planning.point_to_grid_ms += await mapPointBoundsToGrid(fallbackEntries);
      for (const entry of entries) {
        entry.horizontalOverflowRequired =
          entry.rightBoundSource !== 'trailing-hidden' &&
          entry.hasText &&
          entry.rightColumn < MAX_COLUMNS - 1;
      }
      plans = precomputedPlannedItems(entries);
    } else {
      ExperimentUi.setPhase('Checking trailing hidden rows/columns');
      const trailing = await collectTrailingHiddenBounds(sheetNames);
      planning.trailing_hidden_bounds_ms += trailing.elapsed_ms;

      const bounds = await collectContentAndShapeBounds(
        sheetNames,
        trailing.entries
      );
      planning.content_shape_ms += bounds.elapsed_ms;
      planning.point_to_grid_ms += await mapPointBoundsToGrid(bounds.entries);

      if (config.strategy === 'C') {
        await applyVisibleStrategyCMargin(bounds.entries, 10);
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
        visible_shape_count: entry.visibleShapeCount,
        right_bound_source: entry.rightBoundSource,
        bottom_bound_source: entry.bottomBoundSource,
        trailing_hidden_column_start: entry.trailingHiddenColumnStart,
        trailing_hidden_row_start: entry.trailingHiddenRowStart,
        overflow_probes: entry.overflowProbes
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

      const result = config.strategy === 'F'
        ? await renderPrecomputedPlannedBatch(
            workbookName,
            batch,
            batchIndex,
            batchCount
          )
        : await renderAddressBatch(
            workbookName,
            batch,
            batchIndex,
            batchCount
          );

      renderTotal += result.render_ms;
      persistTotal += result.persist_ms;
      planning.overflow_probe_ms += Number(result.analysis_ms || 0);
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
      capture_plans: plans,
      captures: persistedCaptures,
      precomputed_wsl_rust_visual_plan: rustVisualPlan,
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
