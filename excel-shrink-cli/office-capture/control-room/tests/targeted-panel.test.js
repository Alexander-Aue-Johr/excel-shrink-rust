import assert from 'node:assert/strict';

// Exercise the panel's initial empty state without a browser or WebGL context.
class Element {
  children = [];
  attributes = new Map();
  hidden = false;
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
console.log('PASS standalone panel initial update and report reset');
