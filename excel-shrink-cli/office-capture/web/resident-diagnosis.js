/* Targeted counterfactuals on temporary resident workbook copies. */
(function (root) {
  'use strict';

  const MAX_CELLS = 20000;
  const BATCH_CELLS = 128;
  const MAX_HEIGHT_RUNS = 100000;

  function cellKey(cell) {
    if (!Number.isInteger(cell.sheet_index) || cell.sheet_index < 1 ||
        typeof cell.sheet_name !== 'string' || !cell.sheet_name.length ||
        !/^[A-Z]{1,3}[1-9][0-9]{0,6}$/.test(cell.address)) {
      throw new Error('Invalid diagnosis cell identity');
    }
    return `${cell.sheet_index}:${cell.address}`;
  }

  function validateCells(cells, restore = false) {
    if (!Array.isArray(cells) || cells.length > MAX_CELLS) throw new Error('Diagnosis cell budget exceeded');
    const seen = new Set();
    for (const cell of cells) {
      const key = cellKey(cell);
      if (seen.has(key)) throw new Error('Duplicate diagnosis cell');
      seen.add(key);
      if (!restore) continue;
      if (cell.formula !== null && (typeof cell.formula !== 'string' || !cell.formula.startsWith('='))) {
        throw new Error('Invalid source diagnosis formula');
      }
      if (cell.formula === null && !['string', 'boolean', 'number'].includes(typeof cell.value)) {
        throw new Error('Unsupported source diagnosis value type');
      }
      if (typeof cell.value === 'number' && !Number.isFinite(cell.value)) throw new Error('Non-finite diagnosis value');
      if (cell.value_type === 'Error' && cell.formula === null) {
        throw new Error('Literal Excel error restoration is not certified');
      }
    }
    return cells;
  }

  async function calculate(office) {
    await office(async context => {
      context.workbook.application.calculate('Full');
      await context.sync();
    });
    const deadline = Date.now() + 120000;
    while (true) {
      const state = await office(async context => {
        const application = context.workbook.application;
        application.load('calculationState');
        await context.sync();
        return application.calculationState;
      });
      if (state === 'Done') return;
      if (Date.now() >= deadline) throw new Error('Diagnosis calculation did not settle within 120 seconds');
      await new Promise(resolve => setTimeout(resolve, 100));
    }
  }

  async function readCells(cells, office) {
    validateCells(cells);
    const result = [];
    for (let start = 0; start < cells.length; start += BATCH_CELLS) {
      const batch = cells.slice(start, start + BATCH_CELLS);
      const values = await office(async context => {
        const ranges = batch.map(cell => {
          const range = context.workbook.worksheets.getItem(cell.sheet_name).getRange(cell.address);
          range.load('values,formulas,valueTypes');
          return range;
        });
        await context.sync();
        return batch.map((cell, index) => {
          const range = ranges[index];
          const formula = range.formulas[0][0];
          if (cell.has_formula === true && (typeof formula !== 'string' || !formula.startsWith('='))) {
            throw new Error('Original diagnosis formula is unavailable');
          }
          return {...cell, value: range.values[0][0], value_type: range.valueTypes[0][0],
            formula: cell.has_formula === true ? formula : null};
        });
      });
      result.push(...values);
    }
    return result;
  }

  async function restoreCells(cells, office) {
    validateCells(cells, true);
    for (let start = 0; start < cells.length; start += BATCH_CELLS) {
      const batch = cells.slice(start, start + BATCH_CELLS);
      await office(async context => {
        const ranges = batch.map(cell => {
          const range = context.workbook.worksheets.getItem(cell.sheet_name).getRange(cell.address);
          if (cell.formula !== null) range.formulas = [[cell.formula]];
          else range.values = [[typeof cell.value === 'string' ? "'" + cell.value : cell.value]];
          range.load('values,formulas');
          return range;
        });
        await context.sync();
        for (let index = 0; index < batch.length; index++) {
          const source = batch[index], range = ranges[index];
          const matches = source.formula !== null
            ? range.formulas[0][0] === source.formula
            : range.values[0][0] === source.value;
          if (!matches) throw new Error(`Diagnosis restoration changed the source value at ${source.sheet_name}!${source.address}`);
        }
      });
    }
    return cells.length;
  }

  async function restoreHeights(heights, office) {
    if (!Array.isArray(heights) || heights.length > MAX_HEIGHT_RUNS) throw new Error('Diagnosis height budget exceeded');
    for (const run of heights) {
      if (typeof run.sheet_name !== 'string' || !run.sheet_name.length ||
          !Number.isInteger(run.start_row) || !Number.isInteger(run.end_row) ||
          run.start_row < 1 || run.end_row < run.start_row || run.end_row > 1048576 ||
          !Number.isFinite(run.height) || run.height < 0 || run.height > 409.5) {
        throw new Error('Invalid diagnosis height run');
      }
    }
    for (let start = 0; start < heights.length; start += BATCH_CELLS) {
      await office(async context => {
        for (const run of heights.slice(start, start + BATCH_CELLS)) {
          context.workbook.worksheets.getItem(run.sheet_name)
            .getRange(`${run.start_row}:${run.end_row}`).format.rowHeight = run.height;
        }
        await context.sync();
      });
    }
    return heights.length;
  }

  async function execute(command, {office}) {
    if (!command || typeof command.operation !== 'string') throw new Error('Missing diagnosis operation');
    if (command.operation === 'read_cells') {
      return {operation: command.operation, cells: await readCells(command.cells, office)};
    }
    if (command.operation === 'row_heights') {
      return {operation: command.operation, applied_height_runs: await restoreHeights(command.heights, office)};
    }
    if (command.operation === 'restore_cells') {
      const count = await restoreCells(command.cells, office);
      if (command.recalculate === true) await calculate(office);
      return {operation: command.operation, applied_cells: count, explicit_recalculation: command.recalculate === true};
    }
    if (command.operation === 'recalculate') {
      await calculate(office);
      return {operation: command.operation, explicit_recalculation: true};
    }
    throw new Error(`Unknown diagnosis operation: ${command.operation}`);
  }

  async function run(command, helpers) {
    try {
      return await execute(command, helpers);
    } catch (error) {
      // Supplemental diagnostics must not poison the capture client's health
      // and invalidate already completed primary comparisons.
      return {
        operation: command?.operation,
        diagnostic_error: {code: error.code || 'DIAGNOSIS_FAILED', message: error.message || String(error)},
        mutation_state: command?.operation === 'read_cells' ? 'none' : 'possibly_partial'
      };
    }
  }

  const api = {run, validateCells, cellKey};
  root.ResidentDiagnosis = api;
  if (typeof module !== 'undefined' && module.exports) module.exports = api;
}(typeof window !== 'undefined' ? window : globalThis));
