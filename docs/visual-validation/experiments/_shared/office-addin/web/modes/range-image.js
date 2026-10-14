/* global Excel, ExperimentUi, ExperimentEvents, SafeCapture */
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
      sheets.load('items/name,items/visibility,items/position,items/id,items/showGridlines');
      await context.sync();

      return {
        workbookName: workbook.name,
        sheets: sheets.items
          .filter(visibleWorksheet)
          .map((sheet) => ({ name: sheet.name, sheet_index: Number(sheet.position) + 1,
            office_position: Number(sheet.position), worksheet_id: sheet.id, show_gridlines: sheet.showGridlines }))
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

  function horizontalRuleMask(image, background, x0 = 1, x1 = image.width - 1) {
    const candidates = new Uint8Array(image.height);
    x0=Math.max(1,Math.ceil(x0)); x1=Math.min(image.width-1,Math.floor(x1));
    if (x1 - x0 < 16 || image.height < 3) return candidates;

    for (let y = 1; y < image.height - 1; y += 1) {
      const referenceOffset = pixelOffset(image, x0, y);
      if (!pixelDiffers(image, referenceOffset, background)) continue;

      const r = image.data[referenceOffset];
      const g = image.data[referenceOffset + 1];
      const b = image.data[referenceOffset + 2];
      let uniform = true;

      for (let x = x0 + 1; x < x1; x += 1) {
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
    // Thin horizontal rules can be interrupted by content inside the base bounds.
    // Require a uniform band across AT LEAST two visible probe columns; no color-only text deletion.
    const visibleProbeColumns=[];
    for(let c=baseColumn+1;c<=endColumn;c++) if(Number(widths[c])>0) visibleProbeColumns.push(c);
    const probeRules=visibleProbeColumns.length>=2
      ? horizontalRuleMask(image,background,edges[baseColumn+1]+1,edges[endColumn+1]-1)
      : new Uint8Array(image.height);
    const edgeHits=[];
    let ignoredProbeRows=0;
    for(let y=0;y<rules.length;y++) if(probeRules[y]) {if(!rules[y]) ignoredProbeRows++; rules[y]=1;}

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
            if(column===lastVisible && edgeHits.length<8) {
              const o=pixelOffset(image,x,y);
              edgeHits.push({x,y,column,rgba:Array.from(image.data.slice(o,o+4))});
            }
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
      edge_hits:edgeHits,ignored_probe_rule_rows:ignoredProbeRows,
      rule_scope:'full-width and multi-column probe-band thin uniform rules (maximum 4 px)',
      edges
    };
  }

  async function decodePngForOverflow(base64, limits = SafeCapture.DEFAULTS) {
    // Use the same explicit per-sheet budget as planning; do not fall back to
    // the 16 MP default after the user selected 32/64 MP. Check before decoding.
    const header = SafeCapture.png(base64);
    if (!SafeCapture.actualFits(header, limits)) {
      throw SafeCapture.fault('CANVAS_BUDGET', 'PNG exceeds the configured analysis budget.',
        {header, max_pixels:limits.max_pixels, max_edge_px:limits.max_edge_px, stage:'analysis-preflight'});
    }
    let imageElement = null, canvas = null, retained = false;
    try {
      imageElement = new Image();
      await new Promise((resolve, reject) => {
        imageElement.onload = resolve;
        imageElement.onerror = () => reject(SafeCapture.fault('CANVAS_DECODE_FAILED', 'Cannot decode saved capture PNG.'));
        imageElement.src = `data:image/png;base64,${base64}`;
      });
      const width = Number(imageElement.naturalWidth || imageElement.width);
      const height = Number(imageElement.naturalHeight || imageElement.height);
      if (width !== header.width || height !== header.height) {
        throw SafeCapture.fault('CANVAS_SIZE_CHANGED', 'Decoded dimensions disagree with the PNG header.', {header,width,height});
      }
      canvas = document.createElement('canvas');
      canvas.width = width; canvas.height = height;
      const context = canvas.getContext('2d', {willReadFrequently:true});
      if (!context) throw SafeCapture.fault('CANVAS_UNAVAILABLE', 'Canvas 2D context is unavailable.');
      context.drawImage(imageElement, 0, 0);
      const imageData = context.getImageData(0, 0, width, height);
      retained = true;
      return {width,height,data:imageData.data,canvas};
    } finally {
      if (imageElement) {imageElement.onload=null; imageElement.onerror=null; imageElement.src='';}
      if (canvas && !retained) {canvas.width=0; canvas.height=0;}
    }
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
    if (!dataUrl.startsWith('data:image/png;base64,') || comma < 0 || dataUrl.length <= comma + 1) throw SafeCapture.fault('CANVAS_EMPTY_PNG', 'Canvas encoder returned no PNG (including data:,)', {width:canvas.width,height:canvas.height,prefix:dataUrl.slice(0,64)});
    const encoded = dataUrl.slice(comma + 1);
    SafeCapture.png(encoded);
    canvas.width = 0; canvas.height = 0;
    return encoded;
  }

  // Source Context patch: validate and diagnose every Strategy F Range.getImage()
  // payload. Previously only overflow-probe images were decoded in the browser, so a
  // non-overflow sheet could return an empty/short getImage() value and fail later in
  // the host as the context-free error "PNG payload is too short."
  function strategyFPngSummary(base64) {
    const text = typeof base64 === 'string' ? base64 : '';
    const padding = text.endsWith('==') ? 2 : (text.endsWith('=') ? 1 : 0);
    const decodedBytesEstimate = text.length === 0
      ? 0
      : Math.max(0, Math.floor(text.length * 3 / 4) - padding);
    const summary = {
      value_type: typeof base64,
      base64_chars: text.length,
      decoded_bytes_estimate: decodedBytesEstimate,
      base64_prefix: text.slice(0, 32),
      png_signature: false,
      png_width: null,
      png_height: null,
      header_error: null
    };

    if (text.length < 32 || decodedBytesEstimate < 24) return summary;

    try {
      // 32 base64 characters encode exactly the first 24 bytes, which is enough
      // for the PNG signature plus IHDR width/height. Do not decode the whole image
      // merely to validate it; large worksheet captures can be very large strings.
      const header = atob(text.slice(0, 32));
      if (header.length < 24) return summary;
      const byte = (index) => header.charCodeAt(index) & 0xff;
      summary.png_signature =
        byte(0) === 137 && byte(1) === 80 && byte(2) === 78 && byte(3) === 71 &&
        byte(4) === 13 && byte(5) === 10 && byte(6) === 26 && byte(7) === 10;
      if (!summary.png_signature) return summary;

      const readU32 = (offset) => (
        byte(offset) * 0x1000000 +
        byte(offset + 1) * 0x10000 +
        byte(offset + 2) * 0x100 +
        byte(offset + 3)
      );
      summary.png_width = readU32(16);
      summary.png_height = readU32(20);
    } catch (error) {
      summary.header_error = error && error.message ? error.message : String(error);
    }

    return summary;
  }

  function strategyFPngIsValid(summary) {
    return Boolean(
      summary &&
      summary.png_signature &&
      Number(summary.decoded_bytes_estimate) >= 24 &&
      Number(summary.png_width) > 0 &&
      Number(summary.png_height) > 0
    );
  }

  function strategyFImageDetail(workbookName, batchIndex, batchCount, iteration, renderedImage) {
    const item = renderedImage.item;
    return {
      workbook: workbookName,
      batch_index: batchIndex,
      batch_count: batchCount,
      iteration,
      id: item.id,
      sheet: item.sheet,
      address: renderedImage.address,
      bottom_row: item.bottom_row,
      base_right_column: item.base_right_column,
      end_column: item.end_column,
      overflow_probe: Boolean(item.overflow_probe),
      probe_extra: Number(item.probe_extra || 0),
      right_bound_source: item.right_bound_source || null,
      bottom_bound_source: item.bottom_bound_source || null,
      trailing_hidden_column_start: item.trailing_hidden_column_start ?? null,
      trailing_hidden_row_start: item.trailing_hidden_row_start ?? null,
      ...strategyFPngSummary(renderedImage.base64)
    };
  }

  async function retryStrategyFImage(item, address, config) {
    return Excel.run(async (context) => {
      const started = performance.now();
      const traceGetImage = Boolean(
        config &&
        (config.wait_for_start || config.extended_error_logging || config.office_js_debug)
      );
      const range = context.workbook.worksheets
        .getItem(item.sheet)
        .getRange(address);
      if (traceGetImage) {
        context.trace(
          `Strategy F isolated retry BEFORE Range.getImage ` +
          `sheet=${item.sheet} address=${address}`
        );
      }
      const image = range.getImage();
      if (traceGetImage) {
        context.trace(
          `Strategy F isolated retry AFTER Range.getImage ` +
          `sheet=${item.sheet} address=${address}`
        );
      }

      // The initial capture already returned an invalid payload. Pause immediately
      // before the isolated retry is executed so DevTools can inspect item/address.
      if (window.VV_BREAK_ON_ERROR) { debugger; }
      try {
        await context.sync();
      } catch (error) {
        console.error('Strategy F isolated Range.getImage retry failed', {
          error,
          sheet: item.sheet,
          address,
          debugInfo: error && error.debugInfo ? error.debugInfo : null,
          traceMessages: error && Array.isArray(error.traceMessages)
            ? error.traceMessages
            : []
        });
        if (window.VV_BREAK_ON_ERROR) { debugger; }
        throw new Error(
          `Strategy F isolated Range.getImage retry failed for ` +
          `sheet=${JSON.stringify(item.sheet)} address=${address}: ` +
          officeErrorDetail(error)
        );
      }

      return {
        base64: image.value,
        render_ms: performance.now() - started
      };
    });
  }

  // Compact, immutable provenance: capture coordinates are not evidence of content.
  function boundsOnly(item) {
    const fields = ['id','sheet','sheet_index','office_position','address','initial_address',
      'base_right_column','bottom_row','end_column','content_right_column','content_bottom_row',
      'drawing_right_column','drawing_bottom_row','drawing_right_points','drawing_bottom_points',
      'drawing_fallback_required','visible_shape_count','right_bound_source','bottom_bound_source',
      'trailing_hidden_column_start','trailing_hidden_row_start','overflow_probe','probe_extra','overflow_probes'];
    return Object.fromEntries(fields.filter(k => item[k] !== undefined).map(k => [k,item[k]]));
  }
  async function recordBounds(stage, item, detail = {}) {
    const entry = { stage, timestamp_utc:new Date().toISOString(), sheet:item.sheet,
      sheet_index:item.sheet_index ?? null, logical_id:item.id, ...detail };
    (safeSession.state.bounds_history ||= []).push(entry);
    await safeSession.event('bounds_stage', entry);
    await safeSession.checkpoint(stage, {sheet:item.sheet, sheet_index:item.sheet_index ?? null, address:detail.address || item.address});
  }
  // Shared capture safety; content bounds remain the existing strategy's responsibility.
  let safeSession = null;
  let captureRunId = null;
  async function fetch(input, init) {
    const options = {...(init || {})};
    if (captureRunId) options.headers = {...(options.headers || {}), 'X-VV-Run-Id':captureRunId};
    return window.fetch(input, options);
  }
  async function renderPrecomputedPlannedBatch(workbookName, items, batchIndex, batchCount, config) {
    const s=safeSession, initial={...s.timings}, captures=[];
    ExperimentUi.startBatch(batchIndex,batchCount,items.map(x=>x.sheet));
    for (const item of items) {
      const sheetRecord={id:item.id,sheet:item.sheet,sheet_index:item.sheet_index,office_position:item.office_position,
        status:'planning',requested_address:item.address,initial_bounds:boundsOnly(item),tiles:[],overflow_passes:[],
        overflow_mode:config.capture_options?.overflow_mode || 'heuristic'};
      s.state.sheets.push(sheetRecord);
      await s.checkpoint('sheet_started',{sheet:item.sheet,address:item.address});
      try {
        let end=item.end_column, extra=item.probe_extra, accepted=[];
        let overflowDetected=false, passCaptureEnd=end;
        // Convert the configured pixel buffer using this sheet's measured raster scale.
        // Historical/mock calls without the config retain their original column margin.
        if(item.overflow_probe && Number(config.overflow_probe_width_px)>0) {
          const buffer=await CapturePolicy.probeBuffer(s,item,Number(config.overflow_probe_width_px));
          end=buffer.end_column;extra=buffer.extra_visible_columns;sheetRecord.probe_buffer=buffer;
          item.end_column=end;item.probe_extra=extra;item.address=addressA1To(item.bottom_row,end);
          await recordBounds('measured_probe_buffer',item,{...buffer,address:item.address});
        }
        for (let pass=1;pass<=12;pass++) {
          passCaptureEnd=end;
          const rect={r0:0,r1:item.bottom_row+1,c0:0,c1:end+1};
          await recordBounds('overflow_pass_requested', item, {pass,address:SafeCapture.address(rect),
            base_address:addressA1To(item.bottom_row,item.base_right_column), extra_visible_columns:extra});
          const tiles=await s.plan(item.sheet,rect);
          const splitX=tiles.some(t=>t.rect.c0!==0 || t.rect.c1!==end+1);
          let measuredWidths=null;
          if (item.overflow_probe && !splitX) {
            measuredWidths=await s.columnWidths(item.sheet,end);
            // Only unmeasured widths beyond this pass remain XML hints for choosing a wider probe.
            for (let c=end+1;c<MAX_COLUMNS;c++) measuredWidths[c]=Number(item.column_widths[c] || 0);
          }
          let keep=item.base_right_column, done=true, scanned=0;
          const edgeEvidence=[]; let sparseUnresolved=false;
          const result=await s.capturePlan(item.sheet,tiles,`${item.id}-p${pass}`, measuredWidths ? async(base64,tile)=>{
            if(tile.rect.c0!==0 || tile.rect.c1!==end+1) throw SafeCapture.fault('OVERFLOW_CONTEXT_SPLIT','Recovery needed a horizontal split; exact overflow remains inconclusive.',{sheet:item.sheet,rect:tile.rect},true);
            let decoded;
            try {
              const decodeStart=performance.now();
              decoded=await decodePngForOverflow(base64,s.limitsFor(item.sheet));
              const compute=(s.state.compute_profile ||= {});compute.tile_decode_ms=(compute.tile_decode_ms||0)+performance.now()-decodeStart;
              const heuristicStart=performance.now();
              let decision=analyzeRenderOverflow(decoded,measuredWidths,item.base_right_column,end,extra);
              compute.heuristic_scan_ms=(compute.heuristic_scan_ms||0)+performance.now()-heuristicStart;
              if(config.capture_options?.overflow_mode==='selective') {
                if(typeof SelectiveOverflow==='undefined')throw SafeCapture.fault('SPARSE_HELPER_MISSING','Selective helper is unavailable.',{},true);
                const differential=await SelectiveOverflow.verify(s,item,tile,decoded,decision,
                  columnEdgesFromWidths(measuredWidths,end,decoded.width),end);
                decision={...decision,...differential,heuristic_keep_column:decision.keep_column,heuristic_done:decision.done};
                sparseUnresolved=sparseUnresolved||!!differential.unresolved;
              }
              if(!decision.unresolved && decision.keep_column>item.base_right_column)overflowDetected=true;
              keep=Math.max(keep,decision.keep_column); done=done && decision.done;
              scanned+=decision.scanned_pixels;
              if (!decision.done) edgeEvidence.push({tile_id:tile.id,address:tile.address,
                edge_hits:decision.edge_hits,background_rgba:decision.background_rgba,
                ignored_probe_rule_rows:decision.ignored_probe_rule_rows});
              return {keep_column:decision.keep_column,edge_reached:!decision.done,
                scanned_pixels:decision.scanned_pixels,ignored_rule_rows:decision.ignored_rule_rows,
                ignored_probe_rule_rows:decision.ignored_probe_rule_rows,edge_hits:decision.edge_hits,
                background_rgba:decision.background_rgba,rule_scope:decision.rule_scope,selective_checks:decision.checks||[],
                overflow_mode:config.capture_options?.overflow_mode || 'heuristic',unresolved:!!decision.unresolved,reason:decision.reason||null,
                verification_scan_id:decision.scan_id||null,candidate_pixel_rows:decision.candidate_pixel_rows??null,
                candidate_sheet_rows:decision.candidate_sheet_rows||[],verification_group_count:decision.verification_group_count??null};
            } finally {
              if(decoded) { decoded.data=null; if(decoded.canvas) {decoded.canvas.width=0;decoded.canvas.height=0;} }
            }
          } : null);
          sheetRecord.overflow_passes.push({pass,address:SafeCapture.address(rect),keep_column:keep,
            done,scanned_pixels:scanned,tile_count:result.length,horizontal_split:splitX,
            edge_evidence:edgeEvidence,extra_visible_columns:extra,overflow_mode:config.capture_options?.overflow_mode||'heuristic',
            selective_unresolved:sparseUnresolved});
          await recordBounds('overflow_pass_result', item, {...sheetRecord.overflow_passes.at(-1),
            proposed_address:addressA1To(item.bottom_row,keep)});
          if (sparseUnresolved) {
            accepted=result;end=rect.c1-1;item.overflow_checked=false;sheetRecord.status='inconclusive';
            sheetRecord.warning='Selective verification was inconclusive. Original probe bounds are retained; no further expansion or inferred crop.';
            break;
          }
          if (item.overflow_probe && splitX) {
            // Never infer 'no overflow' from tiles that omit potential text sources to their left.
            accepted=result; end=rect.c1-1;
            sheetRecord.status='inconclusive'; sheetRecord.warning='Horizontal split required: full source context for exact overflow not proved. Extra columns are retained.';
            item.overflow_checked=false;
            break;
          }
          if (!item.overflow_probe || done) {
            accepted=result;
            end=item.overflow_probe?keep:end;
            item.overflow_checked=!!item.overflow_probe;
            sheetRecord.status='complete'; break;
          }
          const nextExtra=Math.min(MAX_COLUMNS,Math.max(8,extra*2));
          if (nextExtra > 64) {
            accepted=result; end=rect.c1-1; item.overflow_checked=false;
            sheetRecord.status='inconclusive';
            sheetRecord.warning='Overflow expansion budget reached (64 extra visible columns). The last captured extent is retained; content beyond it is not proven absent.';
            await recordBounds('overflow_expansion_stopped', item, {pass,address:SafeCapture.address(rect),
              reason:'extra-column-budget',next_extra:nextExtra,edge_evidence:edgeEvidence});
            break;
          }
          extra=nextExtra;
          const wider=probeEndByVisibleColumns(item.column_widths,item.base_right_column,extra);
          if(wider<=end) throw SafeCapture.fault('OVERFLOW_NO_PROGRESS','Overflow expansion made no progress',{sheet:item.sheet,end,extra});
          end=wider;
          await s.checkpoint('overflow_expansion',{sheet:item.sheet,end_column:end,pass});
        }
        if(!accepted.length && sheetRecord.status !== 'complete') throw SafeCapture.fault('OVERFLOW_PASS_LIMIT','Overflow did not close within the pass budget',{sheet:item.sheet});
        const detectedEnd=end;
        const exportPolicy=CapturePolicy.outputPolicy({complete:sheetRecord.status==='complete',checked:!!item.overflow_checked,
          base:item.base_right_column,end:passCaptureEnd,probe:!!item.overflow_probe,detected:overflowDetected});
        sheetRecord.export_policy=exportPolicy;
        sheetRecord.detected_bounds=addressA1To(item.bottom_row,detectedEnd);
        end=exportPolicy.end;
        item.end_column=end; item.address=addressA1To(item.bottom_row,end);
        item.overflow_passes=sheetRecord.overflow_passes;
        item.representation='tiles-with-valid-rect-v1';
        // Original probe PNGs remain immutable evidence. Only a fully resolved no-overflow sheet
        // receives physically cropped native PNG derivatives, one tile at a time.
        const finalRect={r0:0,r1:item.bottom_row+1,c0:0,c1:end+1};
        const extent=(await s.geometries(item.sheet,[finalRect]))[0];
        sheetRecord.address=item.address; sheetRecord.extent_points=extent;
        sheetRecord.representation=item.representation;
        for(const sourceTile of accepted) {
          let tile={...sourceTile};
          const width=Math.max(0,Math.min(tile.png_width,Math.round((extent.right-tile.geometry.left)*tile.raster_profile.scale_x)+tile.raster_profile.border_px));
          if(width<=1 || tile.rect.c0>end) { tile.excluded_probe_margin=true; continue; }
          if(exportPolicy.crop && width<tile.png_width) {
            const sourceRect={...tile.rect},sourceGeometry={...tile.geometry};
            const finalTileRect={...tile.rect,c1:Math.min(tile.rect.c1,end+1)};
            tile=await CapturePolicy.exportTile(s,tile,width,SafeCapture.address(finalTileRect));
            tile.source_rect=sourceRect;tile.source_geometry=sourceGeometry;
            tile.rect=finalTileRect;tile.geometry={...sourceGeometry,right:Math.min(sourceGeometry.right,extent.right)};
            tile.content_box_px={x:0,y:0,width,height:tile.png_height};
          }
          tile.valid_rect_px={x:0,y:0,width,height:tile.png_height};
          tile.sheet_offset_px={x:Math.round((tile.geometry.left-extent.left)*tile.raster_profile.scale_x),
            y:Math.round((tile.geometry.top-extent.top)*tile.raster_profile.scale_y)};
          tile.logical_address=item.address; tile.logical_id=item.id;
          sheetRecord.tiles.push(tile); captures.push(tile);
        }
        item.tiles=sheetRecord.tiles;
        await recordBounds('final_capture_bounds', item, {address:item.address,status:sheetRecord.status,
          extent_points:extent,tile_count:sheetRecord.tiles.length,overflow_checked:item.overflow_checked});
        await s.event('sheet_completed',{sheet:item.sheet,address:item.address,status:sheetRecord.status,tile_count:sheetRecord.tiles.length});
      } catch(e) {
        sheetRecord.status='failed'; sheetRecord.error=SafeCapture.errorInfo(e);
        await s.event('sheet_failed',{sheet:item.sheet,error:sheetRecord.error});
        if(!await s.continueAfterSheetFailure(item.sheet,e)) { s.state.status='failed'; await s.checkpoint('fatal_error',{sheet:item.sheet}); throw e; }
      }
      await s.checkpoint('sheet_finished',{sheet:item.sheet,status:sheetRecord.status});
    }
    const delta=k=>s.timings[k]-initial[k];
    ExperimentUi.completeBatch(batchIndex,batchCount,items.length,delta('render_ms'),delta('persist_ms'));
    await s.event('batch_completed',{batch_index:batchIndex,sheets:items.map(x=>x.sheet),tile_count:captures.length,
      geometry_ms:delta('geometry_ms'),render_ms:delta('render_ms'),persist_ms:delta('persist_ms'),analysis_ms:delta('analysis_ms')});
    return {render_ms:delta('render_ms'),persist_ms:delta('persist_ms'),analysis_ms:delta('analysis_ms'),captures};
  }
  async function renderAddressBatch(workbookName,items,batchIndex,batchCount) {
    const s=safeSession, initial={...s.timings}, captures=[];
    ExperimentUi.startBatch(batchIndex,batchCount,items.map(x=>x.sheet));
    for(const item of items) {
      const record={id:item.id,sheet:item.sheet,sheet_index:item.sheet_index,office_position:item.office_position,
        address:item.address,requested_address:item.address,initial_bounds:boundsOnly(item),status:'planning',tiles:[]};
      s.state.sheets.push(record); await s.checkpoint('sheet_started',{sheet:item.sheet,address:item.address});
      try {
        await recordBounds('capture_requested',item,{address:item.address});
        const rect=SafeCapture.parse(item.address), plan=await s.plan(item.sheet,rect);
        const tiles=await s.capturePlan(item.sheet,plan,item.id);
        const g=(await s.geometries(item.sheet,[rect]))[0];
        for(const tile of tiles) {
          tile.valid_rect_px={x:0,y:0,width:tile.png_width,height:tile.png_height};
          tile.sheet_offset_px={x:Math.round((tile.geometry.left-g.left)*tile.raster_profile.scale_x),y:Math.round((tile.geometry.top-g.top)*tile.raster_profile.scale_y)};
          tile.logical_address=item.address;
        }
        record.tiles=tiles; record.status='complete'; record.extent_points=g;
        record.representation=item.representation='tiles-with-valid-rect-v1'; item.tiles=tiles;
        captures.push(...tiles);
        if(plan.some(t=>t.rect.c0!==rect.c0)) {record.status='inconclusive';record.warning='Horizontal tile seams/context require pixel-level validation before production use.';}
      } catch(e) {
        record.status='failed'; record.error=SafeCapture.errorInfo(e);
        await s.event('sheet_failed',{sheet:item.sheet,error:record.error});
        if(!await s.continueAfterSheetFailure(item.sheet,e)) {s.state.status='failed';await s.checkpoint('fatal_error',{sheet:item.sheet});throw e;}
      }
      await recordBounds('final_capture_bounds',item,{address:item.address,status:record.status,
        extent_points:record.extent_points,tile_count:record.tiles.length});
      await s.checkpoint('sheet_finished',{sheet:item.sheet,status:record.status});
    }
    const delta=k=>s.timings[k]-initial[k];
    ExperimentUi.completeBatch(batchIndex,batchCount,items.length,delta('render_ms'),delta('persist_ms'));
    return {render_ms:delta('render_ms'),persist_ms:delta('persist_ms'),captures};
  }

  function officeErrorDetail(error) {
    const message = error && error.message ? error.message : String(error);
    const code = error && error.code ? String(error.code) : '';
    let debugInfo = '';
    let traceMessages = '';

    try {
      if (error && error.debugInfo) {
        debugInfo = JSON.stringify(error.debugInfo);
      }
    } catch (_) {
    }

    try {
      if (error && Array.isArray(error.traceMessages)) {
        traceMessages = JSON.stringify(error.traceMessages);
      }
    } catch (_) {
    }

    return [
      message,
      code ? `code=${code}` : '',
      traceMessages ? `traceMessages=${traceMessages}` : '',
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
    const probeRect = SafeCapture.parse(address);
    await safeSession.ensureCalibration(entry.sheet,probeRect);
    const probeGeometry = (await safeSession.geometries(entry.sheet,[probeRect]))[0];
    if (!SafeCapture.fits(probeGeometry,safeSession.limitsFor(entry.sheet))) {
      throw SafeCapture.fault('XOR_PROBE_BUDGET', 'Legacy XOR probe exceeds the safe image budget; no screenshot or blocker mutation was queued. Use Strategy F for the tiled overflow path.', {sheet:entry.sheet,address,axis,geometry:probeGeometry},true);
    }
    await safeSession.checkpoint('xor_probe_started',{sheet:entry.sheet,address,axis});
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

      let blockerWritten = true; // The write is already queued; attempt restoration even when sync throws.
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

  async function execute() {
    const configResponse = await fetch('/config');
    if (!configResponse.ok) throw new Error(await configResponse.text());
    const config = await configResponse.json();
    captureRunId = config.capture_run_id || config.diagnostic_run_id || null;
    const optionsResponse=await fetch('/capture-options.json',{cache:'no-store'});
    if(!optionsResponse.ok)throw new Error('Capture options are unavailable. No unconfigured capture was started.');
    config.capture_options=await optionsResponse.json();
    config.capture_options={...config.capture_options,show_gridlines:false,final_png_policy:'crop-no-overflow-retain-positive-or-unknown',probe_buffer_unit:'calibrated-pixels'};
    if(!['heuristic','selective'].includes(config.capture_options.overflow_mode))throw new Error('Invalid overflow mode.');
    if(config.capture_options.overflow_mode==='selective' && config.strategy!=='F')throw new Error('Selective verification requires Strategy F.');

    const workbookInfo = await getWorkbookAndVisibleSheets();
    const workbookName = workbookInfo.workbookName;
    if (typeof SafeCapture === 'undefined') throw new Error('safe-capture.js is missing. No unguarded screenshot will be attempted.');
    const sheetNames = SafeCapture.selectSheetNames(workbookInfo.sheets,config.capture_options.sheet_names);
    safeSession = new SafeCapture.Session(workbookName, config);
    if (!await safeSession.claim()) { ExperimentUi.setPhase('Another task pane owns this workbook capture.'); return; }
    if (typeof CapturePolicy === 'undefined') throw new Error('capture-policy.js is missing; no unconfigured screenshots were started.');
    safeSession.state.sheet_catalog = workbookInfo.sheets;
    safeSession.state.capture_scope={mode:config.capture_options.sheet_names?'selected-sheets':'all-visible-sheets',
      selected_sheets:sheetNames,visible_sheet_count:workbookInfo.sheets.length,not_selected:workbookInfo.sheets.filter(s=>!sheetNames.includes(s.name)).map(s=>s.name)};
    safeSession.state.capture_policy_version = CapturePolicy.VERSION;
    await CapturePolicy.hideGridlines(safeSession, workbookInfo.sheets);
    safeSession.state.bounds_history = [];
    await safeSession.event('worksheet_catalog', { sheets: workbookInfo.sheets });
    await safeSession.checkpoint('planning_started', { strategy:config.strategy });

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
      safeSession.state.precomputed_wsl_rust_visual_plan = rustVisualPlan;
      await safeSession.checkpoint('rust_bounds_loaded', { schema: loadedPlan.schema });
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

    for (const item of plans) {
      const identity = workbookInfo.sheets.find(x => x.name === item.sheet);
      item.sheet_index = identity ? identity.sheet_index : null;
      item.office_position = identity ? identity.office_position : null;
      item.initial_address = item.address;
      await recordBounds('addin_initial_plan', item, { bounds: boundsOnly(item) });
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
          await recordBounds('used_range_resolved',item,{address:item.address,bounds:boundsOnly(item)});
        }
      }

      const result = config.strategy === 'F'
        ? await renderPrecomputedPlannedBatch(
            workbookName,
            batch,
            batchIndex,
            batchCount,
            config
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
        logical_sheet_count: batch.length,
        capture_count: result.captures.length,
        actual_image_request_batch_size: 1,
        sheets: batch.map((item) => item.sheet),
        render_ms: fixed(result.render_ms),
        persist_ms: fixed(result.persist_ms),
        total_ms: fixed(total),
        amortized_ms_per_image: fixed(total / Math.max(1, result.captures.length)),
        amortized_ms_per_sheet: fixed(total / Math.max(1, batch.length))
      });
    }

    const sheetResults=safeSession.state.sheets;
    const goodSheets=sheetResults.filter(s=>s.status==='complete').length;
    const failedSheets=sheetResults.filter(s=>s.status==='failed').length;
    safeSession.state.summary={sheet_count:sheetResults.length,complete_sheets:goodSheets,failed_sheets:failedSheets,
      accepted_tiles:persistedCaptures.length,rendered_tile_attempts:safeSession.state.attempts.length,
      calibration_images:safeSession.state.calibrations.reduce((n,c)=>n+c.observations.filter(o=>o.png).length,0)};
    safeSession.state.status = sheetResults.length && goodSheets===sheetResults.length ? 'complete' :
      (failedSheets>0 && goodSheets===0 ? 'failed' : 'partial');
    await safeSession.checkpoint('workbook_finished');
    const payload = {
      status: safeSession.state.status,
      representation: 'tiles-with-valid-rect-v1',
      safe_capture: safeSession.state,
      safe_capture_timings: safeSession.timings,
      capture_view: safeSession.state.capture_view,
      capture_policy_version: CapturePolicy.VERSION,
      actual_image_request_batch_size: 1,
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

    ExperimentUi.setPhase(`${safeSession.state.status === 'complete' ? 'Capture complete' : 'Partial / failed sheets retained'} · waiting for host to close workbook`);
  }

  try {
    await execute();
  } catch (error) {
    const detail = error && error.stack ? error.stack : String(error);
    if (safeSession) { safeSession.state.status='failed'; try { await safeSession.checkpoint('fatal_error',{error:SafeCapture.errorInfo(error)}); } catch (_) {} }
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
