import assert from 'node:assert/strict';

// Exercise the panel's initial empty state without a browser or WebGL context.
class Element {
  children = [];
  attributes = new Map();
  hidden = false;
  dataset = {};
  listeners = new Map();
  closeCount = 0;
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  addEventListener(name, listener) { this.listeners.set(name, listener); }
  close() { this.closeCount++; }
  getBoundingClientRect() { return { left: 100, top: 100, right: 500, bottom: 500 }; }
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
const dialog = root.children.find(child => child.className === 'targeted-job-dialog');
assert.equal(dialog.children[0].getAttribute('aria-label'), 'Close job details');
dialog.children[0].listeners.get('click')();
assert.equal(dialog.closeCount, 1);
const click = dialog.listeners.get('click');
click({ target: dialog, clientX: 200, clientY: 200 });
click({ target: jobs, clientX: 0, clientY: 0 });
assert.equal(dialog.closeCount, 1);
for (const [clientX, clientY] of [[50, 200], [550, 200], [200, 50], [200, 550]]) {
  click({ target: dialog, clientX, clientY });
}
assert.equal(dialog.closeCount, 5);
console.log('PASS standalone panel initial update and report reset');
