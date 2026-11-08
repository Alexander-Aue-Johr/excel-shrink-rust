import assert from 'node:assert/strict';
import { createRetainedFormulaInspector } from '../formula-inspector.js';

// A small DOM double keeps this presentation test independent of a browser API.
class Element {
  constructor(tag, text = '') {
    this.tag = tag;
    this.textContent = text;
    this.children = [];
    this.listeners = new Map();
    this.selectedIndex = 0;
    this.classList = { toggle() {} };
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  setAttribute() {}
  addEventListener(event, listener) { this.listeners.set(event, listener); }
}

globalThis.document = {
  createElement: tag => new Element(tag),
  createTextNode: text => new Element('text', text),
};
globalThis.fetch = () => { throw new Error('Retained formula inspection must remain offline'); };

function descendants(element) {
  return [element, ...element.children.flatMap(descendants)];
}

const formula = 'IF(A1>RAND(),"<script>",0)';
const start = formula.indexOf('RAND');
const diagnosis = {
  evidence: 'saved_compared_states', changed_value_cells: 1, examined_cells: 1,
  volatile_paths: 1, coverage_complete: true,
  examples: [{ status: 'path_found', path: [
    { sheet: 'Data', cell: 'B1', left_value: '1', right_value: '0', formula: 'A2' },
    { sheet: 'Data', cell: 'A2', left_value: '1', right_value: '0', formula,
      volatile_span: [start, start + 6] },
  ] }],
};
const root = createRetainedFormulaInspector({ left: 'OR', right: 'SDR' }, diagnosis);
const nodes = descendants(root);
assert.equal(nodes.filter(node => node.className === 'formula-cell').length, 2);
assert.ok(nodes.some(node => node.textContent === 'Data!B1'));
assert.ok(nodes.some(node => node.textContent === 'OR: 1 → SDR: 0'));
assert.equal(nodes.find(node => node.className === 'formula-volatile-call').textContent, 'RAND()');
assert.ok(nodes.some(node => node.tag === 'text' && node.textContent.includes('<script>')));
const toggle = nodes.find(node => node.tag === 'button');
toggle.listeners.get('click')();
assert.ok(descendants(root).some(node => node.className === 'formula-volatile-call pulse-paused'));

const unavailable = createRetainedFormulaInspector({}, { reason: 'No retained explanation.' });
assert.ok(descendants(unavailable).some(node => node.textContent === 'No retained explanation.'));
assert.ok(descendants(unavailable).some(node => node.textContent.includes('does not prove absence')));
const environment = createRetainedFormulaInspector({ left: 'OR', right: 'SDR' }, {
  evidence: 'saved_compared_states', changed_value_cells: 1, examined_cells: 1,
  volatile_paths: 0, environment_paths: 1, coverage_complete: true,
  examples: [{ status: 'path_found', dependency_kind: 'environment', path: [{
    sheet: 'Data', cell: 'A1', left_value: 'left path', right_value: 'right path',
    formula: 'CELL("filename")', volatile_span: null, environment_span: [0, 16],
  }] }],
});
assert.equal(descendants(environment).find(node => node.className === 'formula-volatile-call').textContent,
  'CELL("filename")');
assert.ok(descendants(environment).some(node => node.textContent.includes('1 filename/environment dependency paths')));
const enriched = createRetainedFormulaInspector({ left: 'OR', right: 'SDR' }, {
  evidence: 'saved_compared_states', changed_value_cells: 1, examined_cells: 1,
  volatile_paths: 0, examples: [],
  source_supplement: {
    evidence: 'prepared_source_cached_values', environment_paths: 1,
    note: 'Source values only.', examples: [{ status: 'path_found', path: [{
      sheet: 'Data', cell: 'A13', formula: 'CELL("filename")', environment_span: [0, 16],
      left_value: 'source path', right_value: 'source path',
    }] }],
  },
});
const enrichedNodes = descendants(enriched);
assert.ok(enrichedNodes.some(node => node.textContent === 'Additional source evidence'));
assert.ok(enrichedNodes.some(node => node.textContent === 'Source cached value: source path'));
assert.ok(enrichedNodes.some(node => node.textContent.startsWith('1 changed saved cell values')));
assert.ok(!enrichedNodes.some(node => node.textContent === 'OR: source path → SDR: source path'));
console.log('PASS offline formula paths, exact call highlight, text safety and pulse control');
