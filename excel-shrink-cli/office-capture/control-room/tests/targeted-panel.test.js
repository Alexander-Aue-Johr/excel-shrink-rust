import assert from 'node:assert/strict';

// Exercise the panel's initial empty state without a browser or WebGL context.
class Element {
  children = [];
  attributes = new Map();
  hidden = false;
  dataset = {};
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  addEventListener() {}
  setAttribute(name, value) { this.attributes.set(name, value); }
  getAttribute(name) { return this.attributes.get(name) ?? null; }
}

globalThis.document = {
  fullscreenElement: null,
  createElement() { return new Element(); },
  addEventListener() {},
};
globalThis.matchMedia = () => ({ matches: false, addEventListener() {} });
globalThis.IntersectionObserver = class {
  observe() {}
  unobserve() {}
};

const { createTargetedPanel } = await import('../targeted-panel.js');

const root = new Element();
const panel = createTargetedPanel(root);
panel.updateMany([]);
assert.equal(root.hidden, true);
panel.updateMany([null, {}]);
assert.equal(root.hidden, true);
panel.reset();
assert.equal(root.hidden, true);
panel.updateMany([]);
assert.equal(root.hidden, true);
const first = { run_id: 'first', workbook: 'first.xlsx', started_at: 1, status: 'running', lanes: [] };
panel.updateMany([first]);
const jobs = root.children.find(child => child.className === 'targeted-job-list');
const firstRow = jobs.children[0];
panel.updateMany([{ ...first, revision: 2, status: 'complete', finished_at: 3 },
  { ...first, run_id: 'second', workbook: 'second.xlsx', started_at: 4 }]);
assert.equal(jobs.children[0], firstRow);
assert.equal(jobs.children.length, 2);
assert.equal(firstRow.dataset.active, 'false');
assert.equal(jobs.children[1].dataset.active, 'true');
console.log('PASS standalone panel initial update and report reset');
