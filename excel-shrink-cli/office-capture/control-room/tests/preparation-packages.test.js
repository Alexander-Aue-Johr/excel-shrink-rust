import test from 'node:test';
import assert from 'node:assert/strict';
import { importRequest, installPreparationPackages } from '../preparation-packages.js';

test('import controls preserve paths and require explicit baseline adoption', () => {
  const fields = {
    package: { value: ' G:\\Prepared corpus\\package ' },
    source_manifest: { value: ' G:\\Selected sources.json ' },
    output: { value: ' V:\\New imported pairs ' },
    generation_choices: { value: '' },
    adopt_packaged_baseline: { checked: false },
  };
  const form = { elements: { namedItem: name => fields[name] } };
  assert.deepEqual(importRequest(form), {
    package: 'G:\\Prepared corpus\\package',
    source_manifest: 'G:\\Selected sources.json',
    output: 'V:\\New imported pairs',
    generation_choices: null,
    adopt_packaged_baseline: false,
  });
  fields.generation_choices.value = ' G:\\choices.json ';
  fields.adopt_packaged_baseline.checked = true;
  assert.equal(importRequest(form).generation_choices, 'G:\\choices.json');
  assert.equal(importRequest(form).adopt_packaged_baseline, true);
  fields.output.value = ' ';
  assert.throws(() => importRequest(form), /new import directory/);
});

test('package paging remains usable and imported pairs require a separate review action', async () => {
  function element() {
    return {
      value: '', checked: false, disabled: false, hidden: false,
      listeners: new Map(), children: [],
      addEventListener(name, handler) { this.listeners.set(name, handler); },
      append(...children) { this.children.push(...children); },
      replaceChildren(...children) { this.children = children; },
    };
  }
  const ids = ['preparation-import', 'preparation-export', 'preparation-summary',
    'preparation-items', 'preview-preparation', 'preparation-previous',
    'preparation-next', 'use-preparation'];
  const nodes = Object.fromEntries(ids.map(id => [id, element()]));
  const fields = Object.fromEntries(['package', 'source_manifest', 'output',
    'generation_choices', 'adopt_packaged_baseline'].map(name => [name, element()]));
  Object.assign(fields.package, { value: 'package' });
  Object.assign(fields.source_manifest, { value: 'sources.json' });
  Object.assign(fields.output, { value: 'new-import' });
  nodes['preparation-import'].elements = { namedItem: name => fields[name] };
  nodes['preparation-export'].elements = { namedItem: () => element() };
  nodes['preparation-next'].disabled = true;
  const previousDocument = globalThis.document;
  globalThis.document = {
    getElementById: id => nodes[id],
    createElement: () => element(),
    querySelectorAll: () => [...Object.values(nodes), ...Object.values(fields)],
  };
  const calls = [];
  const reviewed = [];
  try {
    installPreparationPackages({
      api: async (path, body) => {
        calls.push({ path, body });
        if (path.endsWith('/preview')) return {
          selected: 201, offset: body.offset, counts: { complete: 201 },
          items: [{ name: '<untrusted>', source_id: 'source', status: 'complete', candidates: [] }],
        };
        return { imported: 1, configuration: { source_a: 'baseline', source_b: 'optimized' } };
      },
      message: () => {},
      onImported: configuration => reviewed.push(configuration),
    });
    await nodes['preview-preparation'].listeners.get('click')();
    assert.equal(nodes['preparation-next'].disabled, false);
    assert.equal(nodes['preparation-previous'].disabled, true);
    await nodes['preparation-next'].listeners.get('click')();
    assert.equal(calls[1].body.offset, 100);
    assert.equal(nodes['preparation-previous'].disabled, false);
    nodes['preparation-import'].listeners.get('submit')({ preventDefault() {} });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(nodes['use-preparation'].hidden, false);
    assert.equal(reviewed.length, 0);
    assert.equal(calls.some(call => call.path === '/api/run'), false);
    nodes['use-preparation'].listeners.get('click')();
    assert.equal(reviewed[0].source_b, 'optimized');
    nodes['preparation-import'].listeners.get('change')();
    assert.equal(nodes['use-preparation'].hidden, true);
    assert.equal(nodes['preparation-next'].disabled, true);
  } finally {
    globalThis.document = previousDocument;
  }
});
