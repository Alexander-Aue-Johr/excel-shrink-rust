'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const diagnosis = require('../web/resident-diagnosis.js');

function fixture(entries) {
  const values = new Map(entries.map(cell => [cell.address, {...cell}]));
  const heights = new Map();
  let calculations = 0;
  function range(address) {
    const rowRange = /^[0-9]+:[0-9]+$/.test(address);
    const cell = values.get(address) || {address, value: '', formula: null, type: 'Empty'};
    if (!rowRange) values.set(address, cell);
    return {
      load() { return this; },
      get values() { return [[cell.value]]; },
      set values(value) {
        const scalar = value[0][0];
        cell.value = typeof scalar === 'string' && scalar.startsWith("'") ? scalar.slice(1) : scalar;
        cell.formula = null;
      },
      get formulas() { return [[cell.formula ?? cell.value]]; },
      set formulas(value) { cell.formula = value[0][0]; },
      get valueTypes() { return [[cell.type]]; },
      format: {set rowHeight(value) { heights.set(address, value); }}
    };
  }
  const application = {calculationState: 'Done', load() {}, calculate(kind) {
    assert.equal(kind, 'Full'); calculations++;
  }};
  const office = async action => action({workbook: {application,
    worksheets: {getItem(name) { assert.equal(name, 'Data'); return {getRange: range}; }}},
    sync: async () => {}});
  return {office, values, heights, get calculations() { return calculations; }};
}

const identity = (address, hasFormula = false) => ({sheet_index: 1, sheet_name: 'Data', address, has_formula: hasFormula});

test('reads exact formulas separately from text that begins with equals', async () => {
  const state = fixture([
    {address: 'A1', value: '=literal', type: 'String', formula: null},
    {address: 'B1', value: 3, type: 'Double', formula: '=SUM(1,2)'}
  ]);
  const result = await diagnosis.run({operation: 'read_cells', cells: [identity('A1'), identity('B1', true)]}, state);
  assert.equal(result.cells[0].formula, null);
  assert.equal(result.cells[0].value, '=literal');
  assert.equal(result.cells[1].formula, '=SUM(1,2)');
  assert.equal(state.calculations, 0);
});

test('restores whitespace, typed constants and formulas cumulatively without undoing heights', async () => {
  const state = fixture([]);
  await diagnosis.run({operation: 'row_heights', heights: [{sheet_name: 'Data', start_row: 1, end_row: 9, height: 18}]}, state);
  const cells = [
    {...identity('A1'), formula: null, value: ' \t\u00a0', value_type: 'String'},
    {...identity('B1'), formula: null, value: '=literal', value_type: 'String'},
    {...identity('C1'), formula: null, value: true, value_type: 'Boolean'},
    {...identity('D1'), formula: null, value: 0, value_type: 'Double'},
    {...identity('E1', true), formula: '=SUM(1,2)', value: 3, value_type: 'Double'}
  ];
  const result = await diagnosis.run({operation: 'restore_cells', cells, recalculate: true}, state);
  assert.equal(result.applied_cells, 5);
  assert.equal(state.values.get('A1').value, ' \t\u00a0');
  assert.equal(state.values.get('B1').value, '=literal');
  assert.equal(state.values.get('C1').value, true);
  assert.equal(state.values.get('D1').value, 0);
  assert.equal(state.values.get('E1').formula, '=SUM(1,2)');
  assert.equal(state.heights.get('1:9'), 18);
  assert.equal(state.calculations, 1);
});

test('rejects duplicate addresses and uncertified literal Excel errors before mutation', async () => {
  const state = fixture([]);
  assert.throws(() => diagnosis.validateCells([identity('A1'), identity('A1')]), /Duplicate/);
  const error = await diagnosis.run({operation: 'restore_cells', cells: [
    {...identity('A1'), formula: null, value: '#REF!', value_type: 'Error'}
  ]}, state);
  assert.match(error.diagnostic_error.message, /not certified/);
  assert.equal(error.mutation_state, 'possibly_partial');
  assert.equal(state.values.size, 0);
  const heights = await diagnosis.run({operation: 'row_heights', heights: [
    {sheet_name: 'Data', start_row: 0, end_row: 9, height: 18}
  ]}, state);
  assert.match(heights.diagnostic_error.message, /Invalid/);
  assert.equal(state.heights.size, 0);
});

test('Office errors are supplemental result data, not fatal capture client errors', async () => {
  const office = async () => {
    const error = new Error('The worksheet is protected');
    error.code = 'AccessDenied';
    throw error;
  };
  const result = await diagnosis.run({operation: 'read_cells', cells: [identity('A1')]}, {office});
  assert.equal(result.diagnostic_error.code, 'AccessDenied');
  assert.equal(result.mutation_state, 'none');
});
